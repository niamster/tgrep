# Implementation Plan: Replace custom walker with `ignore::WalkBuilder::build_parallel()`

## Context

Benchmarks show tgrep at ~14s vs rg at ~9s on a large monorepo no-match workload. A/B testing confirmed the gap is entirely structural — the custom sequential recursive walker in `walker.rs` (362 lines) plus the hand-rolled .gitignore parser in `patterns.rs` (514 lines) cannot match the parallel traversal of the `ignore` crate (same crate rg uses). All Track 1 micro-optimizations have been exhausted.

## Approach: parallel walk with compatibility-preserving ordering

Replace the recursive `Walker` with `ignore::WalkBuilder::build_parallel()`. The parallel walker dispatches files to worker threads. Each thread greps the file and buffers output. Output remains deterministic, but ordering must be preserved by stable sequence numbers assigned at discovery time, not by a global post-hoc path sort.

The `futures::executor::ThreadPool` is removed — `ignore`'s parallel walker already provides file-level parallelism, making the separate pool redundant.

## Files to modify

| File | Action |
|------|--------|
| `Cargo.toml` | Add `ignore = "0.4"`, remove `futures`, `crossbeam` |
| `src/utils.rs` | Remove `walker`, `filters`, add `parallel_walker` |
| `src/utils/parallel_walker.rs` | **New** — parallel walk + buffered ordered flush |
| `src/main.rs` | Rewire to use `ParallelWalker`, translate filters/ignore carefully to `ignore` primitives |
| `src/utils/walker.rs` | **Delete** |
| `src/utils/filters.rs` | **Delete** |
| `src/utils/grep.rs` | Extract `grep_file` from `Walker::grep` (open, mmap, binary check, dispatch) |
| `src/utils/writer.rs` | Add `BufferedWriter::take` method |
| `src/utils/display.rs` | Keep as-is — `with_writer()` used for per-file buffered display |
| `src/utils/patterns.rs` | Keep for now — still used by `benches/patterns.rs` |
| `tests/cli.rs` | Add integration tests covering subdirectory walks |

## Implementation steps

### Step 1: Add `ignore` dep, extract `grep_file`

Add `ignore = "0.4"` to `Cargo.toml`.

Move `Walker::grep` logic into a standalone function in `grep.rs`:

```rust
pub fn grep_file(grep: &Grep, path: &Path, matcher: &Matcher, display: &Arc<dyn Display>) {
    match Mapped::open(path) {
        Ok(Some(mapped)) => {
            if content_inspector::inspect(&mapped).is_binary() { return; }
            (grep)(Arc::new(mapped), matcher.clone(), display.clone());
        }
        Ok(None) => {
            (grep)(Arc::new(Zero::new(path.to_path_buf())), matcher.clone(), display.clone());
        }
        Err(_) => {
            (grep)(Arc::new(path.to_path_buf()), matcher.clone(), display.clone());
        }
    }
}
```

This is a refactor with no behavior change — existing `Walker::grep` calls delegate to it.

### Step 2: Implement `parallel_walker.rs`

Core structure:

```rust
struct FileResult {
    seq: u64,
    output: Vec<String>,
}

pub fn parallel_walk(
    paths: &[PathBuf],
    grep: Grep,
    matcher: Matcher,
    display: Arc<dyn Display>,
    overrides: Override,
    follow_links: bool,
    print_file_separator: bool,
) {
    let results: Arc<Mutex<Vec<FileResult>>> = Arc::new(Mutex::new(Vec::new()));

    for fpath in paths {
        let walker = WalkBuilder::new(fpath)
            .hidden(false)         // tgrep does NOT skip dotfiles
            .parents(true)         // honor parent .gitignore files
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(follow_links)
            .overrides(overrides.clone())
            .build_parallel();

        walker.run(|| {
            // Per-thread closure — captures clones of grep, matcher, display, results
            let grep = grep.clone();
            let matcher = matcher.clone();
            let display = display.clone();
            let results = results.clone();
            Box::new(move |entry| {
                let entry = match entry { Ok(e) => e, Err(_) => return WalkState::Continue };
                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    return WalkState::Continue;
                }
                let path = entry.into_path();
                let seq = next_seq.fetch_add(1, Ordering::Relaxed);
                let writer = Arc::new(BufferedWriter::new());
                let file_display = display.with_writer(writer.clone());
                grep_file(&grep, &path, &matcher, &file_display);
                if writer.has_some() {
                    results.lock().unwrap().push(FileResult {
                        seq,
                        output: writer.take(),
                    });
                }
                WalkState::Continue
            })
        });
    }

    // Sort and flush in compatibility order
    let mut results = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    results.sort_by_key(|result| result.seq);
    let writer = display.writer();
    for (i, result) in results.iter().enumerate() {
        if print_file_separator && i > 0 { display.file_separator(); }
        for line in &result.output {
            writer.write(line);
        }
    }
}
```

### Step 3: Translate filter/ignore patterns to `ignore` primitives with parity checks

In `main.rs`, build `Override` from existing CLI args:

```rust
let mut override_builder = OverrideBuilder::new(&fpath);
// force_ignore_patterns -> negated overrides
for pattern in &args.force_ignore_patterns {
    override_builder.add(&format!("!{}", pattern))?;
}
// file_filters -> whitelist overrides (only if user specified filters)
if !filter_patterns.iter().all(|p| p == "*") {
    for pattern in &filter_patterns {
        override_builder.add(pattern)?;
    }
}
let overrides = override_builder.build()?;
```

`.git/` force-ignore is unnecessary — `ignore` crate handles it natively.

However, this translation is not "mechanical" until parity is proven. The current code combines:

- parent `.gitignore` discovery
- per-root forced excludes
- separate file-filter matching
- explicit path handling

So this step needs focused tests for `-e`, `-f`, `-t`, nested `.gitignore`, and multiple roots before the old code is deleted.

### Step 4: Update `main.rs` wiring

Replace the `for path in paths` loop body:

- Remove `Walker::find_ignore_patterns_in_parents` only after confirming `WalkBuilder::parents(true)` matches current repository-root behavior
- Remove `Patterns::new` and `Filters::new` (replaced by overrides)
- Remove `WalkerBuilder` construction
- Call `parallel_walk(...)` instead
- Keep the `path_format` / display construction (still needed for output formatting)
- Keep the stdin handling block

Symlink behavior is not a free replacement. The current walker also canonicalizes symlinks, detects loops, and avoids descending into already-covered parent trees. If `ignore` does not match that behavior directly, keep a thin compatibility layer instead of silently changing traversal semantics.

### Step 5: Add `BufferedWriter::take` method

`writer.rs` needs a new method to extract the buffered lines:

```rust
pub fn take(&self) -> Vec<String> {
    let lines = self.lines.lock().unwrap();
    lines.borrow_mut().take().unwrap_or_default()
}
```

### Step 6: Remove old code, update module declarations

- Delete `src/utils/walker.rs` and `src/utils/filters.rs`
- Update `src/utils.rs`: remove `walker`, `filters`, add `parallel_walker`
- Remove `futures` and `crossbeam` from `Cargo.toml`
- Keep `patterns.rs` (still used by benchmark)

### Step 7: Update tests

- Migrate walker unit tests to CLI integration tests in `tests/cli.rs`
- Add a test with nested subdirectories + `.git` + `.gitignore` (exercises parallel walk)
- Existing CLI tests should pass unchanged

## Key design decisions

### Why buffered ordered flush (not global path-sort)?

For the no-match workload (the optimization target), there are zero results to buffer, so memory cost is nil. For match-heavy workloads, memory is proportional to total output size, which is bounded by what would be printed anyway. A sequence-number priority queue is a future optimization if end-of-walk buffering becomes a concern.

### Why remove `futures::ThreadPool`?

With `ignore`'s parallel walker, each file is already dispatched to a worker thread. The separate pool adds overhead with no benefit — parallelism is already at the file level.

### Why keep `patterns.rs`?

Still used by `benches/patterns.rs`. Can be removed in a separate cleanup.

### Output ordering

Current: deterministic traversal-based ordering, including CLI root order.

New: deterministic ordering must remain compatible with current behavior. A global full-path sort is simpler, but it is a behavior change and should not be treated as equivalent.

## Verification

1. `cargo test` — all unit + integration tests pass
2. `cargo build --release` then:
   - `./target/release/tgrep foooxxxyyy ~/dd/dd-source/domains </dev/null` — completes, no hang
   - `hyperfine` A/B vs baseline binary
3. Correctness: `diff` output against baseline binary — should match exactly for representative trees, including nested dirs and multiple CLI roots
4. Feature parity: test `-i`, `-v`, `-l`, `-L`, `-c`, `-A`/`-B`, `-e`, `-f`, `-t`, `--ignore-symlinks`
5. Symlink parity: verify loop detection and ancestor-tree dedup behavior still match the current walker
