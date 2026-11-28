# A tour of Flux

Run `cargo run --locked -- examples/basics/hello.flx` from the repository root,
or install the CLI as shown in the [quickstart](../README.md#quickstart).
Unless noted in the [manifest](manifest.json), run `flux PATH` from the root.

## 1. Expressions and collections

[Hello](basics/hello.flx), [prefix expressions](basics/expressions.flx),
[collections](basics/collections.flx), [guards](basics/guards.flx),
[pipelines](basics/pipeline.flx), [sequence calculus](basics/sequence-calculus.flx).

## 2. Functions and inference

[Named functions and function values](basics/functions.flx),
[recursive sums and matching](basics/trees.flx).

## 3. Tacit programming

[Arithmetic](tacit/arithmetic.flx), [composition](tacit/composition.flx), [forks](tacit/forks.flx),
[hooks](tacit/hooks.flx), [projection](tacit/projection.flx),
[normalisation](tacit/normalisation.flx), [predicates](tacit/predicates.flx),
[direct calls](tacit/callable.flx).

[Multiple arguments](tacit/nary.flx), [partial application and captures](tacit/partial.flx),
[generalized forks](tacit/generalised-forks.flx).

## 4. Mathematics

[Wide arithmetic](mathematics/exact.flx), [combinatorics](mathematics/combinatorics.flx),
[Euclid](mathematics/euclid.flx), [Fibonacci](mathematics/fibonacci.flx),
[primes](mathematics/primes.flx), [Horner evaluation](mathematics/polynomial.flx),
[rationals](mathematics/rational.flx), [rounding](mathematics/rounding.flx),
[approximation](mathematics/approximation.flx).

[Array calculus](mathematics/array-calculus.flx), [matrix products](mathematics/matrix-products.flx),
[axis reductions](mathematics/axis-reduction.flx),
[dense arrays and indexed sequences](mathematics/arrays.flx).

## 5. Statistics and tables

[Description](statistics/descriptive.flx), [percentiles](statistics/percentiles.flx),
[distribution](statistics/distribution.flx), [grouped statistics](statistics/grouped.flx).

[Observations](tables/observations.flx), [monitoring](tables/monitoring.flx),
[joins](tables/joins.flx), [missing values](tables/missing.flx),
[deduplication and union](tables/duplicates.flx), [projection](tables/projection.flx),
[aggregation](tables/aggregation.flx), [catalogue](tables/catalogue.flx),
[open rows](tables/open-rows.flx).

## 6. Libraries, typed failure and I/O

[Text](library/text.flx), [regex](library/regex.flx), [dates](library/dates.flx),
[JSON](library/json.flx), [optionals](library/optionals.flx), [results](library/results.flx).

[Raw CSV](io/csv-grid.flx), [typed CSV](io/csv-typed.flx), [arguments](io/arguments.flx),
[streams](io/streams.flx), [files](io/files.flx).

## 7. Modules

[Geometry](modules/geometry/demo.flx),
[polymorphic transformations](modules/transformations/demo.flx),
[exported tacit callables](modules/callables/demo.flx).

## 8. Applications

- [Instrument calibration](applications/calibration/README.md): typed CSV, exact offsets and duplicate-safe matching.
- [Concordance](applications/concordance/README.md): alphabetical and frequency-ranked word lists.
- [Meteorology](applications/meteorology/README.md): station/date groups with missing and invalid readings.
- [Astronomy](applications/astronomy/README.md): matching synthetic identifiers, not sky positions.
- [Demography](applications/demography/README.md): exact regional shares from a synthetic census.

For a compact introduction to effects, try
[fallible actions](io/fallible-actions.flx); the
[array-and-tacit script](mathematics/array-tacit-script.flx) combines
broadcasting, axis reduction and explicit execution.

## Inputs and verification

The [manifest](manifest.json) records arguments and standard input as well
as expected stdout, exit status, generated files and no-overwrite checks.
For example, run `flux examples/io/arguments.flx -- report 2026`, or pipe
`first`, a blank line, then `last` into the streams example. Use a clean
copy of the examples tree for file-writing specimens: monitoring creates
`summary.csv` and files creates `corpus_files/note.txt`.

`cargo test --locked --test examples` checks the inventory and tests
each program in ordinary and strict HIR modes in isolated directories.
It tests library files through their consumers. For the language rules,
see the [Flux reference](../docs/specification.md).
