# Flux examples

From the repository root:

```sh
cargo run -- examples/permutation.flx
cargo run -- examples/observations.flx
cargo run -- examples/csv-grid.flx
```

`permutation.flx` exercises prefix arithmetic, guarded cases, range reductions,
named function references, and tuple-spread application. `observations.flx`
starts with a typed in-memory table, filters a Boolean mask, and computes a count
and mean. No spreadsheet adapter or special worksheet operator is involved.
`csv-grid.flx` runs the experimental raw CSV reader. Its result is a grid of
strings, preserving the header and quoted cells; the prototype does not yet
coerce input into a named table schema.

See [the language specification](../docs/specification.md) for proposed
adapter-based analysis, joins, reconciliation, time series, and batch workflows.
