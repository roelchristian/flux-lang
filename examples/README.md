# A tour of Flux

Run the commands from the repository root unless an application says
otherwise. Install the CLI with `cargo install --path .`, or replace
`flux` in a command with `cargo run --`.

## 1. Expressions and collections

Start with [hello](basics/hello.flx), then try [prefix expressions](basics/expressions.flx),
[collections](basics/collections.flx), [guards](basics/guards.flx), and [pipelines](basics/pipeline.flx).
Try `flux fmt examples/basics/pipeline.flx` to see the source formatting.

## 2. Functions and inference

[Functions](basics/functions.flx) distinguishes a named callable, which
consumes its fixed number of operands directly, from a function value,
which you apply with `@` or pass to another function. For a declared
callable, `&name` gets its function value. `@@` supplies the elements of a
tuple as a sequence of curried arguments; `@` supplies one value. The inferred
identity is used at two unrelated types.

## 3. Tacit programming

Read [arithmetic](tacit/arithmetic.flx), [composition](tacit/composition.flx),
[forks](tacit/forks.flx), [hooks](tacit/hooks.flx), [projection](tacit/projection.flx),
[normalisation](tacit/normalisation.flx), and [predicates](tacit/predicates.flx).
Inside `&{ ... }`, `.` is identity. `@. g h` applies `h` first, then `g`.
A fork transforms one input twice, then combines the results; a hook uses
identity for the left branch. Pipelines transform values immediately;
tacit blocks build reusable functions. Arithmetic can lift over columns as
specified; custom functions do not automatically lift. Use `@:` to map explicitly.

## 4. Mathematical computation

[Exact arithmetic](mathematics/exact.flx), [combinatorics](mathematics/combinatorics.flx),
[Euclid](mathematics/euclid.flx), [Fibonacci](mathematics/fibonacci.flx),
[primes](mathematics/primes.flx), [Horner evaluation](mathematics/polynomial.flx),
[rationals](mathematics/rational.flx), [rounding](mathematics/rounding.flx), and
[approximation](mathematics/approximation.flx).
Exact division produces rationals, not truncated decimals. Use explicit
quantisation when a finite decimal scale is needed, and convert explicitly
when approximation is intended.

## 5. Statistics and tables

[Description](statistics/descriptive.flx), [percentiles](statistics/percentiles.flx),
[distributions](statistics/distribution.flx), and [grouped statistics](statistics/grouped.flx).

[Observations](tables/observations.flx) and [monitoring](tables/monitoring.flx) work with
typed measurements. Then see [joins](tables/joins.flx), [missing values](tables/missing.flx),
[duplicates and set operations](tables/duplicates.flx), [projection](tables/projection.flx),
[aggregation](tables/aggregation.flx), and the [catalogue](tables/catalogue.flx).
Unlike SQL-style NULL comparisons, absent optional keys match one another.
Repeated keys yield multiple pairs. Empty result tables retain their checked schema.

## 6. Libraries, typed errors and I/O

[Text](library/text.flx), [regular expressions](library/regex.flx), [dates](library/dates.flx),
[JSON](library/json.flx), [optionals](library/optionals.flx), and [results](library/results.flx).
Absence is not failure. Use `!` to execute an action and `^?` or `^=` to
propagate or recover from a typed result; constructing the action does not run it.

[Raw CSV](io/csv-grid.flx), [typed CSV](io/csv-typed.flx), [arguments](io/arguments.flx),
[streams](io/streams.flx), and [files](io/files.flx). The [manifest](manifest.json)
gives deterministic input and command-line arguments, where needed. For
example, run `flux examples/io/arguments.flx -- report 2026`, or pipe
`first`, a blank line, then `last` into the streams example. Use a clean copy
of the examples tree to run the file-writing specimens: monitoring creates
`summary.csv`, and files creates `corpus_files/note.txt`.

## 7. Source modules

Run the [geometry demo](modules/geometry/demo.flx) for qualified nominal types
and annotated public functions. Run the [transformations demo](modules/transformations/demo.flx)
for polymorphic instantiation, constraints, private definitions and first-class exported functions.

## 8. Complete applications

- [Instrument calibration](applications/calibration/README.md): typed CSV, exact offsets, and duplicate-safe matching.
- [Concordance](applications/concordance/README.md): alphabetical and frequency-ranked word lists.
- [Meteorology](applications/meteorology/README.md): station/date groups with missing and invalid readings.
- [Astronomy](applications/astronomy/README.md): matching synthetic object identifiers, not sky positions.
- [Demography](applications/demography/README.md): exact regional shares from a synthetic census.

## Verifying the corpus

`cargo test --locked --test example_corpus` checks all entry points, then runs
both the ordinary and strict HIR executors in clean temporary directories.
The manifest records expected stdout, exit status, input, generated files and
no-overwrite behavior. Libraries are verified through their consumers, and an
unlisted script or an unused library fails the inventory check.
