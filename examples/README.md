# A tour of Flux

Run [Hello](basics/hello.flx) from the repository root:

```sh
cargo run --locked -- examples/basics/hello.flx
```

Once [installed](../README.md#quickstart), run a specimen with `flux PATH`.
Paths and fixture names below assume the repository root unless stated
otherwise. [Inputs and verification](#inputs-and-verification) covers stdin,
arguments and generated files.

## 1. Read and write prefix expressions

Start with [expressions](basics/expressions.flx) and
[collections](basics/collections.flx): the operator comes first and its
arity determines how many operands it takes. Follow with
[guards](basics/guards.flx), [pipelines](basics/pipeline.flx) and
[sequence calculus](basics/sequence-calculus.flx) for branching, staged
transformations and reductions.

## 2. Compose and reuse functions

- [Functions and inference](basics/functions.flx) introduces named
  callables and first-class values. Use `@ f x` for a function-valued
  binding, or declare a direct call with [`name/arity`](tacit/callable.flx).
  [Recursive trees](basics/trees.flx) uses constructor matching.
- Start tacit programming with [arithmetic](tacit/arithmetic.flx) and
  [composition](tacit/composition.flx). Then try [forks](tacit/forks.flx),
  [hooks](tacit/hooks.flx), [projection](tacit/projection.flx),
  [normalisation](tacit/normalisation.flx) and [predicates](tacit/predicates.flx).
  Explore [multiple arguments](tacit/nary.flx),
  [partial application and captures](tacit/partial.flx) and
  [generalized forks](tacit/generalised-forks.flx) when unary functions
  are no longer enough.
- For checked module interfaces, run the consumers for
  [geometry](modules/geometry/demo.flx),
  [polymorphic transformations](modules/transformations/demo.flx)
  and [exported tacit callables](modules/callables/demo.flx).

## 3. Choose a numerical domain

- **Exact or approximate?** Compare [wide integers and exact decimals](mathematics/exact.flx)
  with [binary64 conversion](mathematics/approximation.flx), which converts
  an approximate tenth back to its actual rational value. See also
  [rational arithmetic](mathematics/rational.flx) and
  [explicit half-even rounding](mathematics/rounding.flx).
- **Algorithms.** Try [combinatorics](mathematics/combinatorics.flx),
  [Euclid](mathematics/euclid.flx), [Fibonacci](mathematics/fibonacci.flx),
  [primes](mathematics/primes.flx) and [Horner evaluation](mathematics/polynomial.flx).
  The Mathesis library specimens cover [integer functions](mathematics/integer-functions.flx),
  [polynomials](mathematics/polynomial-operations.flx), [wave sampling](mathematics/waves.flx),
  [numerical roots](mathematics/numerical-roots.flx),
  [integration and approximation](mathematics/integration.flx) and
  [multidimensional mathematics](mathematics/multidimensional-math.flx).

### Dense arrays and linear algebra

- Start with [dense arrays and indexed sequences](mathematics/arrays.flx),
  then [broadcasting](mathematics/array-calculus.flx),
  [axis reductions](mathematics/axis-reduction.flx),
  [matrix products](mathematics/matrix-products.flx) and an
  [array-and-tacit script](mathematics/array-tacit-script.flx).
- **Draft 0.21 Machina II.** [Dense views](mathematics/dense-views.flx)
  builds a dense array from a range, takes a window of a transposed
  view and shows exact widening. [Matrix contraction](mathematics/matrix-contraction.flx)
  checks a matrix entry against a dot product.
- **Draft 0.22 Algebra.** [Linear systems](mathematics/linear-systems.flx)
  solves an integer-input system exactly. Compare with real-valued
  [matrix factorizations](mathematics/matrix-factorizations.flx) and
  [least-squares fitting](mathematics/least-squares.flx), including
  recoverable failures. [Complex rotations](mathematics/complex.flx)
  uses an explicit complex domain.

## 4. Analyse structured observations

- [Observations](tables/observations.flx) and [projection](tables/projection.flx)
  introduce columns and predicates. [Open rows](tables/open-rows.flx)
  reuse a transformation across schemas.
- **Draft 0.23 Tabula.** [Column selection](tables/column-selection.flx)
  filters by a mask in table order, then projects and renames checked fields.
  The explicit sort reorders the result and preserves the observed zero
  and the decimal scale. For stable row order and matching semantics,
  see [joins](tables/joins.flx), [duplicates and union](tables/duplicates.flx),
  [missing values](tables/missing.flx) and [catalogue matching](tables/catalogue.flx).
- Try [description](statistics/descriptive.flx), [percentiles](statistics/percentiles.flx),
  [distributions](statistics/distribution.flx), [grouped statistics](statistics/grouped.flx)
  and [compound-key aggregation](tables/aggregation.flx).
  [Monitoring](tables/monitoring.flx) combines typed ingestion, grouping
  and a no-overwrite CSV export.

## 5. Execute effects and handle failures

Start with [fallible actions](io/fallible-actions.flx) to see when
`!` runs a suspended action. Compare [optionals](library/optionals.flx)
with [typed results and recovery](library/results.flx), then try
[raw CSV](io/csv-grid.flx), [typed CSV](io/csv-typed.flx) and
[JSON](library/json.flx).
Other library specimens cover [text](library/text.flx), [regex](library/regex.flx)
and [dates](library/dates.flx); system specimens cover
[arguments](io/arguments.flx), [streams](io/streams.flx) and [files](io/files.flx).

## 6. Run a complete application

- [Instrument calibration](applications/calibration/README.md): exact offsets, duplicate records and a no-overwrite report. Run from its own directory.
- [Concordance](applications/concordance/README.md): alphabetical and frequency-ranked word lists.
- [Meteorology](applications/meteorology/README.md): station/date groups with missing and invalid readings.
- [Astronomy](applications/astronomy/README.md): match synthetic catalogues by identifier and constellation.
- [Demography](applications/demography/README.md): exact regional shares from a synthetic census.

## Inputs and verification

```sh
flux examples/io/arguments.flx -- report 2026
printf 'first\n\nlast\n' | flux examples/io/streams.flx
```

Use a clean copy of the example tree when running programs that write:
monitoring creates `summary.csv` at the root, the files example creates
`corpus_files/note.txt` there, and calibration creates `findings.csv`
in its working directory. The [manifest](manifest.json) records working
directories, inputs, expected output and no-overwrite checks.

Use `flux check PATH` for static checking, or `flux run-hir PATH` for
strict execution. Compare a specimen in optimized and generic typed modes:

```sh
flux run-hir examples/tables/column-selection.flx
FLUX_OPTIMIZE=0 flux run-hir examples/tables/column-selection.flx
```

The corpus uses default budgets. When adapting an example for larger
data, opt in to higher [host limits](../docs/implementation.md#runtime-policy)
explicitly, for example:

```sh
FLUX_MAX_ELEMENTS=1000000 FLUX_MAX_WORK=5000000 \
  flux run-hir examples/mathematics/dense-views.flx
```

This changes limits, not the fixture. Allow for memory before increasing
sizes. Examples show semantics; see the
[benchmark guide](../docs/implementation.md#performance) for measurements.

`cargo test --locked --test examples` checks the inventory and runs
each program in ordinary and strict HIR modes, with optimization both on
and off, in isolated directories.
Library files are checked through their consumers. See the
[Flux reference](../docs/specification.md) for language rules.
