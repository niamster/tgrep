# Performance Investigation Plan

## Goal

Reduce `tgrep` overhead on large file trees without regressing correctness or deterministic output ordering.

The reference workload used so far is a no-match search over a large source tree with:

```bash
tgrep fooxxxyyy <large-tree>
```

This is useful because it exposes traversal and per-file setup costs without conflating them with output volume.

## Current State

The branch already contains the low-risk walker cleanup from PR #19:

- `file_type()` during directory scans instead of eager `metadata()`
- `Vec + sort_unstable()` instead of hot-path `BTreeMap`
- lazy `BufferedWriter` allocation for no-match files

Those changes should remain the baseline for future experiments.

## Measurements So Far

Benchmark results have been noisy across sessions, but the most useful recent runs on the current code shape are:

- `find <large-tree> -type f > /dev/null`: about `7.35s real`
- `rg fooxxxyyy <large-tree>`: about `32.17s real` in the same noisy window
- `tgrep fooxxxyyy <large-tree>` before the latest experiments: about `22.12s real`
- `tgrep fooxxxyyy <large-tree>` after the latest experiments: about `20.98s real`

The exact numbers vary with machine and filesystem state, so comparisons should be made from back-to-back runs only.

## Instrumentation

There is now opt-in phase timing behind:

```bash
TGREP_PROFILE=1 ./target/release/tgrep ...
```

This reports aggregate timings for the main phases:

- directory scanning
- ignore checks
- `.gitignore` parsing
- metadata lookup
- small-file reads
- `mmap`
- prefilter
- fuzzy grep
- exact grep
- display

The instrumentation is currently implemented in:

- [src/utils/timing.rs](/Users/dmytro.milinevskyi/dev/tgrep/src/utils/timing.rs)
- [src/utils/grep.rs](/Users/dmytro.milinevskyi/dev/tgrep/src/utils/grep.rs)
- [src/utils/walker.rs](/Users/dmytro.milinevskyi/dev/tgrep/src/utils/walker.rs)
- [src/main.rs](/Users/dmytro.milinevskyi/dev/tgrep/src/main.rs)

## What We Learned

### 1. Regex work is not the bottleneck on the no-match workload

Before the latest prefilter changes, the profile showed:

- `file.mmap`: dominant aggregate cost
- `file.inspect_binary`: also large
- `file.metadata`: meaningful but secondary
- `walk.gitignore`: larger than expected
- `grep.fuzzy` and `grep.total`: relatively small

That meant further matcher micro-optimizations were unlikely to move wall time much.

### 2. A literal prefilter is correct but not enough by itself

A conservative plain-literal byte prefilter was added for:

- non-empty patterns
- no regex metacharacters
- case-sensitive searches only
- non-inverted searches only

This removed almost all actual grep work on the no-match case, but only improved wall time slightly because the dominant cost was still per-file setup.

### 3. Avoiding `mmap` for most small no-match files helps

A small-file shortcut was added:

- if a safe literal prefilter is available
- and the file is below a size threshold
- read the file once and reject it before `mmap` if the literal is absent

This reduced wall time from about `22.12s` to about `20.91s` in the profiled run.

### 4. Separate `.gitignore` probes were real overhead

Local `.gitignore` parsing was fused into the directory scan instead of probing `path/.gitignore` separately for every directory.

That reduced aggregate `walk.gitignore` time from about `8.37s` to about `0.27s`.

Wall time stayed roughly flat after that specific change, which indicates the removed work was real but not on the final critical path anymore.

### 5. The remaining dominant costs are file-open/read and traversal

On the latest profile, the main costs are now:

- `file.read_small`
- `walk.read_dir`
- `file.metadata`
- `file.mmap` on the subset of files that still reach mapping

Actual grep work is close to zero on this workload.

## Experiments Already Tried

### Kept

1. Low-risk walker cleanup
2. Opt-in phase instrumentation
3. Conservative plain-literal prefilter
4. Small-file read-before-`mmap` shortcut
5. Fused local `.gitignore` parsing into directory scans

### Tried And Not Worth Keeping As-Is

1. Traversal/search split with top-level parallel path collection, global sort, then grep

Why it was dropped:

- did not show a convincing win on the updated base
- increased complexity
- delayed first output
- did not address the actual dominant costs shown by profiling

## Next Branch Experiments

These should be attempted in separate branches so they can be compared independently.

### Branch A: Single-Open Small-File Path

Hypothesis:

The current small-file prefilter still opens and reads the file, then later may open it again through the normal reader path if it passes. A dedicated in-memory reader for small files should remove duplicate file-open work.

Proposed changes:

- add a `LinesReader` implementation backed by owned in-memory bytes or string content
- when a small file is read for prefilter and the prefilter passes, reuse that buffer for grep instead of reopening or remapping
- try to remove the separate `metadata()` call where possible by deriving enough from the open/read path

Success criteria:

- lower `file.metadata`
- lower `file.mmap`
- lower wall time on the no-match workload

### Branch B: No-Metadata Open Path

Hypothesis:

The explicit metadata call per file is still avoidable overhead.

Proposed changes:

- open the file once
- use file handle metadata only when needed, ideally from the same open handle
- restructure `Walker::grep` so the decision tree uses one open path instead of metadata first, then open/mmap/read

Success criteria:

- lower `file.metadata`
- possibly fewer syscalls overall
- measurable wall-time reduction

### Branch C: `ignore`/`walkdir` Prototype

Hypothesis:

The remaining traversal overhead may be better handled by a battle-tested walker and ignore engine than by continued custom walker tuning.

Proposed changes:

- prototype replacing the custom directory walk and ignore matching layer with `ignore::WalkBuilder`
- preserve deterministic final ordering explicitly if needed
- keep the current grep pipeline unchanged for the first prototype

Success criteria:

- lower `walk.read_dir`
- lower ignore-related overhead
- equal or simpler correctness model

Risks:

- integration complexity
- ordering semantics need explicit handling
- behavior around symlinks and parent ignore discovery must be checked carefully

### Branch D: Adaptive Threshold Tuning

Hypothesis:

The current small-file shortcut threshold is only a first guess.

Proposed changes:

- benchmark several thresholds for the read-before-`mmap` shortcut
- possibly make the threshold conditional on search mode

Success criteria:

- identify whether the current threshold is close to optimal
- avoid growing complexity if the gains are marginal

### Branch E: Profiling Cleanup And Stable Harness

Hypothesis:

The benchmark environment has been noisy enough that a stable local harness will save time and reduce false conclusions.

Proposed changes:

- add a reproducible benchmark script or documented command sequence
- run `find`, `rg`, and `tgrep` back-to-back
- keep `TGREP_PROFILE=1` available for phase inspection

Success criteria:

- easier apples-to-apples comparisons
- less churn from environmental noise

## Recommended Order

1. Branch A: single-open small-file path
2. Branch B: no-metadata open path
3. Branch E: stable benchmark harness if repeated measurements remain noisy
4. Branch C: `ignore`/`walkdir` prototype if traversal is still the limiting factor
5. Branch D: threshold tuning only after the larger structural experiments

## Notes

- Keep deterministic output ordering.
- Be cautious about changes that delay first output unless they produce a clear win.
- Use the same back-to-back benchmark sequence for each branch.
- Prefer measuring the same moment with `find`, `rg`, and `tgrep` rather than comparing results from different sessions.
