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

## Aesthetic revision

`pipeline.flx` applies the prefix pipeline to an inclusive range. The same
immutable stage value is referenced twice in the filter; the printed sum is
`337965`.

`monitoring.flx` is the end-to-end typed CSV example for the aesthetic
prototype. Run from the repository root so it resolves
`examples/monitoring.csv`. The four inputs include a negative valid reading
and a positive invalid reading; both are filtered out. The remaining North
group has count 2, exact mean and median 15, and population standard deviation
3. Execution prints the result and explicitly writes `summary.csv`; use a
fresh output location before rerunning when the adapter refuses to overwrite.
Use the tested implementation notes for adapter and numeric limits.
