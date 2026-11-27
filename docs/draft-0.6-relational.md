# Flux Draft 0.6: Relational operations

This revision adds joins and explicit table transformations to the typed HIR.
It retains [exact numeric domains](draft-0.5-numerics.md), typed optionals
and errors, ordered table schemas, and explicit effects. No operation below
reads or writes a file.

## Joins

In these signatures `#a` and `#b` are tables with checked row types.
`keyL` and `keyR` are function values. Each accepts the full row on its
own side and returns the same hashable key type `k`.

| Operation | Checked operands | Result |
|---|---|---|
| `*= keyL keyR left right` | `(a->k) (b->k) #a #b` | `#(a,b)` |
| `*? keyL keyR left right` | same | `#(a,?b)` |
| `df.semi keyL keyR left right` | same | `#a` |
| `df.anti keyL keyR left right` | same | `#a` |

An inner join emits a pair for every matching combination. A left join
emits each match with a present right row, or exactly one pair with an
absent right row when a left row has no match. Left rows are emitted in
their input order, and matches for each left row are emitted in the order
of the right input. Duplicates are not silently collapsed. Semi-join
retains each left row whose key has at least one right match, once per
left occurrence; anti-join retains each left occurrence with no match.
Neither semi-join nor anti-join emits right rows.

Output schemas are checked before execution. An empty result keeps its
row type, including the two row types (and right optional) of a join.
Paired rows deliberately avoid implicit field-name collision resolution.
For example, `.1 pair` is the left row and `.2 pair` the right row or
optional right row. Use a typed projection to name fields for a flat report.

## Equality and grouping

Join and group keys follow checked Flux equality, including tuple and
optional keys with hashable contents. A tuple compares component by
component. A missing optional key matches another missing optional key;
it does not match a present key. This is Flux equality, not SQL NULL
semantics. To omit such rows, explicitly drop missing keys before joining.
An unexpectedly unmatched row and a matched row whose amount is absent
are distinct optional layers.

Equal exact numbers have the same key identity regardless of integer
storage size or decimal display scale. Finite approximate signed zeros
compare equal and hash consistently when used as allowed keys. Static
schema and type rules still apply: the join does not convert two
different key types to a common type on the fly.

Grouping with `=:` and aggregating with `df.agg` accept the same compound
keys. Group order follows the first appearance of each key; rows within
each group retain source order. The key callback is evaluated once for
each row it keys, and join output cardinality never changes that count.
Joins key the right input first, then the left input. A nonempty input
still supplies its key callbacks when the other input is empty.
Reference-runtime bounds cap materialized results separately from input
row counts.

## Row and nullable transformations

| Operation | Checked operands and result |
|---|---|
| `df.union left right` | `#a #a -> #a` |
| `df.distinct table` | `#a -> #a` |
| `df.select project table` | `(a->b) #a -> #b`, where `b` is a row |
| `df.dropna key table` | `(a->?k) #a -> #a` |
| `df.fill default values` | `a [?a] -> [a]` |
| `df.present values` | `[?a] -> [a]` |

Union is a set union in stable first-occurrence order: visit all left
rows, then all right rows, retaining only the first occurrence of each
equal full row. Distinct applies the same rule within one table. Neither
changes the retained row's value; for example, the first of two
equal decimal rows keeps its display scale. A union preserves the left
schema's field order. Closed structural rows can match by named fields
regardless of declaration order, while distinct nominal row identities
remain different types.

Projection takes an explicit function value. Its checked output row
provides field names, order, and element types even for empty input. It
can project and rename fields together, or construct new derived fields.
It does not infer a schema from sample rows. `df.dropna` retains rows whose
selected key is present, without moving or unwrapping other fields.
`df.fill` substitutes the one supplied value only for absences and
preserves sequence length. `df.present` removes absences and preserves
the order of present payloads. A present zero is not absence.

Set operations require equality-comparable rows; functions and suspended
actions are not silently treated as comparable fields. Existing
concatenation `+:` remains ordered and preserves duplicates.
Operations that retain or combine same-schema rows preserve their checked
schema on empty output. Nominal row identities are not interchangeable
simply because their fields happen to be named alike.

Window functions, pivots, as-of joins, and columnar execution are not in
this revision.

## Reconciliation example

Copy the files in `examples/reconciliation` to a working directory and run
`flux run-hir reconcile.flx` there. The synthetic ledger and statement CSVs
produce five exceptions, including a decimal discrepancy of `0.25`, records
present on only one side, duplicate source records and a matched record with
a missing statement amount. Genuine zero amounts match and do not appear
as missing. The output is an ordered `report.csv`, created without
overwriting an existing file.
