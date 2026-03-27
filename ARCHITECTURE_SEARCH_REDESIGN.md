# Search Architecture Redesign

## Context

Recent performance experiments improved `tgrep`, but only incrementally.

The strongest clean-branch result so far combined:

- walker-side `.gitignore` handling from directory scans
- removal of the standalone `metadata()` pass before `mmap`

That improved a representative no-match large-tree workload from roughly `23.42s` to roughly `22.19s`.

### Benchmark Baseline (2026-03-27)

A fresh benchmark on the post-PR-19 codebase against a large monorepo subtree (~hundreds of thousands of files) with pattern `foooxxxyyy` (no matches, exercises traversal + open + fuzzy-reject hot path):

| Tool  | Wall time | User  | System  |
|-------|-----------|-------|---------|
| tgrep | 13.92s    | 2.62s | 37.97s  |
| rg    | 9.09s     | 1.34s | 91.39s  |

Key takeaway: `rg` is ~1.5x faster wall-clock despite using ~2.4x more total system time. This means `rg` wins almost entirely through **parallelism** — it issues far more concurrent syscalls across threads. `tgrep`'s user-space time is already low; the bottleneck is sequential syscall throughput, not CPU work.

### Conclusion

Local tuning has mostly reached diminishing returns. The remaining cost is dominated by the core per-file processing pipeline:

- open file
- classify text vs binary
- read or map file contents
- search contents

And critically, by the **sequential traversal of directories**, which serializes the syscall-heavy pipeline.

This note proposes a structural redesign for that pipeline.

## Current Problems

### 1. Traversal, classification, search, and output are tightly coupled

In the current code, `src/utils/walker.rs` is responsible for:

- recursive traversal
- `.gitignore` handling
- symlink handling
- file classification decisions
- launching grep work
- partial output-order coordination

That makes local tuning possible, but it makes larger performance improvements harder because one stage cannot evolve independently of the others.

### 2. Every file pays too much setup cost

The current path is effectively:

1. find file
2. fetch metadata
3. open or map file
4. classify binary vs text
5. run grep
6. render output

For a large tree with mostly non-matches, this means the expensive parts are paid before there is strong evidence that the file is worth fully processing.

### 3. The search path is too line-oriented too early

Line extraction is appropriate for rendering results, but it is not the best primitive for deciding whether a file should be fully searched in the first place.

### 4. Sorted output constrains concurrency

The current design keeps output deterministic, but traversal and execution are not cleanly separated from ordering. That limits opportunities for deeper parallelism.

### 5. Double stat per file

`walk_dir` calls `entry.file_type()` on each directory entry, then `grep()` calls `fs::metadata()` again on the same path to obtain the file size for `mmap`. On macOS, `file_type()` reads from the dirent structure (no extra syscall), but `metadata()` is a full `stat()`. Since `Mapped::new` could let `mmap` auto-detect the file size from the fd's `fstat` (issued internally by the kernel during `mmap`), the explicit `metadata()` call is redundant.

### 6. Sequential directory traversal

`walk_dir` processes subdirectories in a sequential loop. Only files within a single directory are parallelized via `grep_many`. The benchmark confirms this: `rg` achieves 91s of system time (many concurrent syscalls across threads) vs `tgrep`'s 38s. The wall-clock gap is almost entirely explained by this difference in traversal parallelism.

### 7. Walker clone and gitignore probe on every directory

`walk_dir` clones the `Walker` struct and attempts to open `.gitignore` on every directory entry, even when no `.gitignore` exists. The clone itself is cheap (Arc reference bumps), but the failed file open is a wasted syscall per directory.

### 8. Regex engine used for literal fuzzy pre-check

`fuzzy_grep` calls `regexp.shortest_match(map)` to pre-filter files before line iteration. For a literal pattern like `foooxxxyyy`, this runs the full regex engine over the entire mmap'd content. A `memmem`-based literal search (already available in `patterns.rs` via libc FFI) would be faster for this common case.

### 9. Remaining wins are structural, not profile-level

`Cargo.toml` already enables `lto = true` and `codegen-units = 1` in `[profile.release]`.

That matters because it removes one obvious "free win" from the table: the remaining gap to `rg` is no longer plausibly explained by missing release-profile tuning. The remaining work is architectural: traversal parallelism, search-path specialization, and reducing unnecessary per-file setup.

## Decision

Adopt a staged search architecture with explicit subsystem boundaries:

1. `Walk`
2. `Classify`
3. `Search`
4. `Order`
5. `Render`

The high-level goal is:

- traverse quickly
- classify files cheaply
- fully search only likely candidates
- preserve deterministic output ordering

## Proposed Architecture

### 1. Walk Layer

Responsibility:

- recursive traversal
- ignore handling
- symlink policy
- file discovery
- stable file sequencing

Recommended direction:

- replace the custom recursive walker with `ignore::WalkBuilder`
- assign a stable sequence number to every discovered file
- emit `FileTask { seq, path }`
- preserve current symlink behavior explicitly rather than assuming `follow_links(true)` is a drop-in replacement

Why:

- this aligns the traversal layer with the same family of tooling used by `rg`
- it removes a large amount of custom ignore and recursion logic
- it makes later parallelism easier
- it keeps ordering semantics separate from execution semantics, which is required for safe parallelism

### 2. Classify Layer

Responsibility:

- open file once
- read a small prefix
- quickly determine:
  - empty file
  - likely binary
  - likely text
  - plain-literal impossible match
  - needs full search

Proposed interface:

```rust
struct FileTask {
    seq: u64,
    path: PathBuf,
}

enum FileDecision {
    Skip,
    EmptyText,
    PrefixText(PrefixBuffer),
    NeedsFullSearch(FileHandleState),
}
```

Key design rule:

- classification must be cheap
- classification must reuse the same open handle or already-read bytes where possible
- classification is an optimization stage, not a rendering stage; regex/context modes may still need a full downstream path

This is the main architectural change that the experiments point toward.

### 3. Search Layer

Responsibility:

- execute the actual search strategy chosen for the file

This should split by query shape:

- `LiteralSearchEngine`
- `RegexSearchEngine`

And by file state:

- search from prefix-buffered content when enough data is already loaded
- search from a full in-memory buffer for small files
- search from `mmap` or stream for larger files

Important principle:

- do not default every search to the same path

A plain literal and a full regex with context should not pay the same setup cost.

### 4. Order Layer

Responsibility:

- preserve deterministic output ordering independent of execution order

Proposed model:

- walker assigns `seq`
- workers search files in parallel
- results are emitted as `SearchResult { seq, payload }`
- an order coordinator flushes results strictly in `seq` order

This gives:

- deterministic output
- more freedom for parallel traversal and search
- cleaner separation between execution and user-visible ordering

Important constraint:

- `seq` must represent compatibility order, not convenience order
- global path-sort is not an equivalent replacement for today's behavior when multiple roots or symlinked trees are involved

### 5. Render Layer

Responsibility:

- turn matches into user-visible output

This should remain line-oriented, but only after the search layer has decided the file actually contains something worth rendering.

That means:

- byte-oriented or buffer-oriented search first
- line formatting second

## Proposed Module Split

One possible shape:

- `src/walk/`
  - `mod.rs`
  - `task.rs`
  - `ignore.rs`
- `src/classify/`
  - `mod.rs`
  - `prefix.rs`
  - `binary.rs`
- `src/search/`
  - `mod.rs`
  - `literal.rs`
  - `regex.rs`
- `src/order/`
  - `mod.rs`
  - `buffer.rs`
- `src/render/`
  - current display-oriented code can migrate here over time

This does not need to happen in one rewrite, but it should be the target shape.

## Compatibility Constraints

Any redesign should preserve these user-visible behaviors unless the project explicitly chooses to change them:

- multiple input roots are processed in CLI argument order
- output remains deterministic under parallel execution
- symlink traversal keeps the current loop and ancestor-duplication protections
- `-e`, `-f`, `-t`, parent `.gitignore`, and explicit file arguments continue to compose correctly

Those constraints should shape the implementation plan. They are not follow-up polish.

## Execution Flow

Target runtime flow:

1. walk emits `FileTask`
2. classify opens file once and reads a small prefix
3. classify decides:
   - skip
   - emit empty result
   - search buffered content directly
   - continue with full search path
4. search engine runs in parallel
5. results are buffered by sequence number
6. renderer flushes in deterministic order

## Pre-Redesign Optimizations

Before committing to the full staged redesign, several targeted fixes can reduce the gap with `rg` within the existing architecture. These are worth doing first because they are low-risk, independently valuable, and inform whether the full redesign is necessary.

### O1. Release profile tuning

Add to `Cargo.toml`:

```toml
[profile.release]
lto = true
codegen-units = 1
```

Expected impact: small but free. Tighter inlining across crate boundaries, especially for the `memchr`, `regex`, and `content_inspector` hot paths.

### O2. Eliminate redundant metadata syscall

Remove the `fs::metadata()` call in `Walker::grep()`. Instead of passing a pre-computed `len` to `Mapped::new`, let `MmapOptions` derive the file size from the fd (which it does internally via `fstat` when no explicit length is set). This eliminates one `stat()` per file.

### O3. Skip Walker clone when no .gitignore

In `walk_dir`, check for `.gitignore` existence before cloning the walker. When no `.gitignore` is found (the common case in deep subtrees), reuse `self` directly instead of cloning and re-wrapping `ignore_patterns` in a new `Arc`.

### O4. Parallel directory traversal — requires structural redesign

Attempted: parallelize subdirectory walks using `std::thread::scope` with a depth limit, buffering each subtree's output in a `BufferedWriter` and flushing in sorted order.

Results (no-match large-tree workload, on top of O1–O3 baseline of 11.93s):

| Approach | Wall time | User | System |
|----------|-----------|------|--------|
| O1–O3 (sequential walk) | 11.93s | 2.47s | 45.19s |
| O4 depth=3 | 14.16s | 3.26s | 31.44s |
| O4 depth=1 | 13.07s | 2.69s | 29.21s |

Both parallel walk variants regressed wall time despite reducing total system time. The root cause is that `std::thread::scope` imposes synchronization barriers at each directory level — the parent must wait for ALL children before continuing. This limits effective parallelism and adds OS thread creation/joining overhead that outweighs the benefit.

Additionally, the `futures::executor::ThreadPool` cannot be used for recursive directory spawning (causes thread pool starvation deadlock), forcing the use of OS threads.

Conclusion: parallel directory traversal cannot be bolt-on to the current recursive `walk_dir` architecture. It requires a producer-consumer design where the walk emits file paths into a shared work queue and independent worker threads consume them — which is the Track 2 redesign (specifically, `ignore::WalkBuilder::build_parallel()` or equivalent).

### O5. Literal pre-filter bypass

When the pattern compiles to a pure literal (no regex metacharacters), use `memmem` (already available via libc FFI in `patterns.rs`) instead of `regexp.shortest_match()` for the fuzzy pre-check in `generic_grep`. This avoids regex engine overhead on the most common case for the no-match workload.

### O6. Avoid parents Vec reallocation

Replace the `parents.to_owned()` + push pattern in `walk_dir` with a stack-like structure or pass a reference to a shared growable buffer, avoiding a full `Vec<PathBuf>` clone at every directory level.

## Migration Plan

### Prototype Findings

The first direct classify-layer prototype was attempted on top of the plain post-PR-19 baseline.

Result:

- it regressed badly on the no-match large-tree workload
- the main issue was adding extra per-file prefix I/O before removing enough of the old heavy path

Implication:

- a classify layer is still the right long-term direction
- but it must be introduced on top of a better file-processing base, not layered naively over the original `metadata -> map -> inspect -> grep` flow
- for a repository that is mostly text, prefix-only classification is not a strong standalone optimization unless it replaces enough downstream work

Revised guidance:

- start future classify-layer work from the stronger combined branch shape
- specifically, build it on top of the branch that already:
  - removes the standalone `metadata()` probe
  - folds local `.gitignore` handling into directory scans

### Phase 1

Introduce explicit `FileTask` and `SearchResult` types without changing behavior much.

Goal:

- separate traversal identity from rendering identity

### Phase 2

Replace the custom walker with `ignore::WalkBuilder`, but keep the current grep path.

Goal:

- modernize traversal and ignore handling first

### Phase 3

Add a real classify layer:

- open once
- prefix read
- cheap binary detection
- literal prefilter when applicable

Goal:

- make early skips materially cheaper than full search

### Phase 4

Split literal and regex search engines.

Goal:

- stop forcing all query types through the same pipeline

### Phase 5

Add ordered result buffering by sequence id.

Goal:

- unlock more parallelism while preserving sorted output

## Alternatives Considered

### Continue micro-optimizing the current design

Rejected because the experiment branches showed only modest gains. The remaining bottleneck is structural.

### Give up deterministic ordering

Rejected because deterministic output appears to be a real project requirement.

### Switch directly to a complete ripgrep-like architecture in one step

Not rejected in principle, but too large and risky as a single change. Incremental migration is safer.

## Consequences

### Positive

- better alignment with the actual bottlenecks
- cleaner subsystem boundaries
- more room for meaningful parallelism
- easier future experimentation with search engines and file strategies

### Negative

- more code motion
- higher implementation complexity
- temporary coexistence of old and new paths during migration

## Experiment Results

### Track 1 (O1–O6)

All Track 1 optimizations were implemented and benchmarked. A/B testing with hyperfine against the pre-optimization baseline (same cache state, same machine) showed:

| Variant | Wall time | User | System |
|---------|-----------|------|--------|
| Baseline (pre-O1) | 14.22s | 2.68s | 31.32s |
| O1–O3 | 14.22s | 2.46s | 34.62s |
| O1–O3+O5+O6 | 14.52s | 2.45s | 29.25s |

O1–O3 reduce user-space CPU time by ~8% (LTO + fewer syscalls + fewer clones), but wall time is unchanged because this workload is I/O-bound. O5 (literal pre-filter) provides no benefit — the regex engine already uses literal optimizations internally. O6 (parents Vec reallocation) is negligible.

O4 (parallel directory traversal) regressed wall time when attempted via `std::thread::scope` (see above).

Earlier measurements showing ~14% improvement from O1–O3 were artifacts of filesystem cache warming, not code changes.

### Conclusion

O1–O3 are kept as code quality improvements (cleaner APIs, less redundant work, better release builds) but they do not close the gap with `rg` on this I/O-bound workload. O5–O6 are dropped as they add complexity without measurable benefit.

## Recommendation

The gap with `rg` (~14s vs ~9s) is entirely structural and requires the Track 2 architectural migration:

1. walker replacement (`ignore::WalkBuilder`) — this is the clear first priority, as it provides parallel traversal via a producer-consumer design rather than recursive thread spawning
2. classify layer on top of the stronger combined file path
3. split literal and regex search engines
4. ordered result buffering by sequence id

Track 1 micro-optimizations have been exhausted. No further local tuning can close the gap.
