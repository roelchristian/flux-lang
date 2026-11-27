# P1 relational and tabular computation: implementation review

Base: the standard-library and combinators branch (PR #7). All operations
use checked, typed HIR. The earlier paired-row join notation remains
unchanged: `*=` pairs both rows, and `*?` stores an optional right row.
Keys are checked at both call sites. Hash tables retain matching right-row
positions while the evaluator emits left rows in input order; buckets are
not iterated to determine observable output. Duplicate matches produce the
full multiplicity. Semi/anti joins retain the checked left schema.

Stable union and distinct preserve first encounter order. The result of
projection/renaming carries its checked layout even when the input is
empty. Compound optional and tuple keys work in joins and grouping.
Missing values are treated explicitly by `df.dropna`, `df.fill`, and
`df.present`; zero is never conflated with absence.

## Independent review and revisions

The independent pass compared all four join modes against a simple
cross-product oracle, including randomized duplicate datasets, optional
keys, exact decimal scale, and signed floating zero. It found two union
defects: different field order could produce a runtime error after a
successful check, and function-valued rows could pass set-union checking.
Union now accepts reordered structurally compatible fields while preserving
the left layout, and requires real structural equality for row values.
Both defects were independently retested. The output cardinality limit
allowed 99,856 matching pairs and rejected a join exceeding 100,000.

The [synthetic financial reconciliation](../examples/reconciliation/)
passed under both default and strict CLI. Seven rows on each side produce
eight inner pairs and nine left rows. The emitted CSV records an exact
+0.25 mismatch, one ledger-only and one statement-only key, one duplicate
key with 2-by-2 source counts, and a matched statement row with no amount.
The actual zero reconciles. A rerun does not overwrite the first report.

Rust 1.85: 281 all-feature/all-target tests, formatting, strict Clippy, and
locked release build passed. Twelve end-to-end relational tests, 22 separate
independent fixtures, 10 prior differential specimens and 1,020 exact
numeric oracle specimens passed.

## Reference performance

Seven interleaved release samples on a pinned Linux CPU (including CLI
startup). Each source builds two equally sized half-overlapping tables,
then executes inner, left, and anti joins.

| Rows per input | Median |
| ---: | ---: |
| 1,000 | 4.19 ms |
| 6,000 | 17.63 ms |
| 12,000 | 35.98 ms |

These are bounded reference-interpreter workloads, not a columnar engine
benchmark. Rolling/window functions, pivots, temporal joins, and production
columnar execution are deliberately later.

## Final stack review

Three independent passes checked cross-module typed HIR calls, relational
ordering and schema evidence, and exact-number/typed-error behavior across
library and table boundaries. Imported typed joins and projection callbacks,
empty schemas, curried table operations, CSV exact-value round trips and JSON
error provenance through result combinators matched the checked semantics.

Two findings were revised before merge. Table equality now compares named
fields rather than their declaration order, so deduplication handles nested
tables with equivalent schemas; display still follows the enclosing table's
column order. JSON decoding now limits cumulative exact-number expansion to
one million digits per document, in addition to individual number and
document limits. The regression suite checks both sides of this limit.

For pathological nested values, the structural deduplication fallback counts
row comparisons, not every nested comparison. This remains a performance
limitation of the reference interpreter.
