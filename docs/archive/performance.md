# Flux interpreter review

This review concerns the Draft 0.3 aesthetic interpreter, not the original
starter or a production columnar engine. The objective was to shorten real
analysis and checking workloads without changing strict evaluation order.

## Changes

- The lexer dispatches punctuation to a longest-first list for the leading
  byte. A regression enumerates every original and new spelling.
- Grouping and distinct use a hash index for scalar values, while preserving
  order of first appearance explicitly. All exact numeric representations of
  the same value share one key. Floating-point signed zeros share a key, and
  exact values stay separate from floating values. Structural values use
  equality comparison, with the existing operation-size limit intact.
- Correctness review also tested schema-directed reads, mixed-order and
  duplicate CSV headers, escaped fields, malformed rows, optional blanks and
  no-clobber output. Structural input schemas no longer fall back to a string
  grid. A present optional whose serialization would be confused with missing
  data is rejected rather than silently written; the output is not created.

## Reproduce

Use release builds with the same Rust version and settings:

```sh
python3 benchmarks/compare.py /path/to/before/flux target/release/flux --runs 9
```

The Python harness creates temporary fixtures, warms both binaries, alternates
execution order, and verifies the stdout of every run before recording the
time. Reported medians are process-level elapsed time, including startup and
CSV ingestion where relevant. They do not isolate allocator or VM costs.

## Results

Release builds on the same Linux x86-64 host with Rust 1.85, nine paired runs.

| Workload | Before (ms) | After (ms) |
|---|---:|---:|
| numeric pipeline (100,000 values) | 42.27 | 36.87 |
| lex/parse/check (4,000 declarations) | 19.89 | 11.57 |
| typed CSV + aggregate (6,000 rows, 32 keys) | 11.89 | 12.24 |
| typed CSV + aggregate (6,000 rows, 6,000 keys) | 117.36 | 32.34 |

High-cardinality aggregation was about 3.6× faster; the 32-key case changed little.
These synthetic results do not generalize to every data shape.

## Next priorities

1. Cache incremental REPL parse/check state. An independent debug-build test
   with 300 declarations measured approximately 433 ms for incremental stdin
   submissions versus 4.5 ms for checking one file. The measurement includes the
   REPL's current full-session rebuilds and is not a release comparison.
2. Investigate shared immutable collection storage. Pipeline references and
   repeated projections can deep-copy table or sequence values. Change the
   representation before changing evaluation order.
3. Bound environment growth with explicit closure captures and safe scope
   lifetimes. Current frame IDs may escape in closures or suspended actions;
   simply popping frames is not sound.
4. Consider pre-indexed delimiters and scoped type maps for larger modules.
   Range fusion and streaming come later: changing when arithmetic errors occur
   would break the specified eager semantics.
