# ADR 0001: Grep Performance Plan and Benchmarking Prerequisites

## Status

Accepted

## Context

`tgrep` needs a performance-oriented architecture change with one non-negotiable functional requirement:

- output must be sorted lexicographically by file path

A second requirement is desirable but subordinate to sorting:

- emit output as soon as possible once matches are known, while still respecting lexicographic order

The current implementation mixes traversal, search, buffering, and output coordination. That makes it difficult to reason about:

- true output ordering guarantees
- time to first emitted match
- throughput under parallel search
- whether a change is actually faster

Before changing the architecture, the project needs an explicit target design and benchmark coverage that measures both throughput and latency.

## Decision

We will execute the work in three stages:

1. Define the target architecture and measurement strategy in an ADR.
2. Add benchmarks and deterministic benchmark corpora before changing the hot path.
3. Refactor the search pipeline only after a measurable baseline exists.

## Ordering Rule

The new architecture must preserve this invariant:

- files are emitted in global lexicographic order

This means output order is determined by sorted file paths, not by traversal order and not by worker completion order.

The implementation may search files out of order and in parallel, but the emitter must not print file `N` until all lexicographically earlier files are resolved.

## Target Architecture

The target design is a three-stage pipeline:

1. Enumerator
   - discovers candidate files
   - applies ignore and file-filter rules
   - produces a globally lexicographically sorted stream or list of files
   - assigns each file a monotonic sequence number derived from sorted order

2. Worker pool
   - searches files in parallel
   - returns either `NoMatch` or `Match` results keyed by sequence number
   - is allowed to search ahead of the current output cursor

3. Ordered emitter
   - owns stdout
   - tracks the next sequence number eligible for emission
   - buffers out-of-order completed results until all earlier files are resolved
   - emits the earliest ready file immediately

This architecture replaces directory-local batching with a global coordination model.

## Benchmark Prerequisites

Before the refactor, the project needs deterministic benchmark inputs and benchmark coverage at three levels.

### Deterministic corpora

Benchmark corpora must be generated from a fixed seed or deterministic recipe so that results are comparable across revisions.

The generated datasets should cover:

- many tiny files with sparse matches
- many tiny files with dense matches
- medium files with regular matches
- large files with rare matches
- trees with `.gitignore` usage
- cases where early lexicographic files are slow or negative and later files match quickly

### Benchmark layers

1. Traversal benchmarks
   - measure file discovery, filtering, and ignore processing
   - exclude regex search cost where possible

2. Search benchmarks
   - measure grep over a precomputed sorted file list
   - isolate search and result assembly from tree walking

3. End-to-end CLI benchmarks
   - measure the real binary on generated corpora
   - capture both total runtime and first-output latency

## Metrics

The benchmark suite must report or make it easy to derive:

- total wall-clock runtime
- time to first stdout byte
- time to first emitted match
- total files searched
- total matches emitted
- throughput in files per second or bytes per second where useful

For the ordered emitter design, latency metrics are first-class. A change that improves total runtime but worsens first emitted match latency is not automatically an improvement.

## Initial Implementation Plan

The work will proceed in the following order:

1. Add benchmark corpus generation helpers.
2. Add Criterion benchmarks for traversal-oriented and search-oriented measurements.
3. Add at least one process-level benchmark harness for first-output and first-match latency.
4. Record a baseline on the current implementation.
5. Refactor the architecture toward a global sorted enumerator, parallel workers, and a single ordered emitter.
6. Compare the refactor against the baseline before iterating further.

## Alternatives Considered

### Refactor first, benchmark later

Rejected because the redesign is specifically about performance and latency tradeoffs. Without a baseline, it is too easy to optimize the wrong stage.

### Preserve current traversal order as output order

Rejected because the required functional invariant is lexicographic file ordering.

### Optimize only the regex hot path

Rejected as a first step because current behavior indicates that traversal, buffering, and output coordination are also on the critical path.

## Consequences

This decision adds benchmark infrastructure before user-visible architecture changes.

That increases short-term work, but it reduces the risk of:

- shipping a refactor that regresses latency
- optimizing traversal while output buffering remains the bottleneck
- debating architecture changes without numbers

The refactor that follows this ADR should be evaluated against benchmark evidence, not only code inspection.
