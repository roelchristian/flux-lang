# Draft 0.7 example corpus: verification

Six parallel implementation lanes built the introductory and tacit examples,
mathematics and statistics, the calibration audit, relational workloads,
standard-library and module specimens, and complete applications. All use the
existing compiler and runtime unchanged.

Separate review from the implementation authors checked the calibration CSV
against the expected rows, including a header-only run and one-sided duplicate
inputs; checked raw join multiplicity, nullable zero and decimal equality;
and inspected introduction, module examples, links and the harness.

The review tightened tutorial coverage: true polymorphic function-value
exports, an inferred identity in the introduction to functions, astronomy
cross-matching on a composite key, and exact demographic shares. It also
made all no-overwrite rerun stdout expectations explicit and fixed
normalisation of root-relative generated paths in the inventory check.

The manifest covers 52 executable specimens. Both CLI execution modes must
match each golden result and are run in separate temporary directories. The
harness also runs static checking, verifies that generated files match
fixtures and that reruns do not alter them, and calculates reachable source
modules so an omitted example or orphan library cannot pass unnoticed.
Mathematics and statistics expectations were calculated separately using
Python integer arithmetic, fractions, decimals and statistics. The five
legacy example workloads remain pinned as private regression inputs so the
old and new interpreters can still be compared without adapting either.

See the [tour](../examples/README.md) to find a specimen or reproduce a result.
