# Flux changelog

The [language reference](docs/specification.md) describes current behavior.
This log summarizes material changes. Earlier text and design revisions
are available through [the repository's commit history](https://github.com/roelchristian/flux-lang/commits/main/);
obsolete specifications are not kept as duplicate files.

## Unreleased

- Consolidate shared Rust internals and streamline the reference and example
  index without changing the language. Group integration tests by topic.

## Draft 0.23

- Add Tabula column buffers, validity masks and ordered table views behind
  checked HIR. Retain the row-backed typed path for reference execution.
- Share proven projections and decode eligible typed CSV directly into
  columns without changing schema, cell diagnostics or row ordering.
- Export `df.sort`, `df.concat`, `df.take` and `df.drop` as aliases of
  existing operators; use `df.select` for checked column renames.
## Draft 0.22

- Add `lin` for exact determinants, rank, solutions, inverses and matrix
  powers, and checked real norms, LU, thin QR, Cholesky and least squares.
  Shape and numerical failures have typed `linerr` results.
- Add finite binary64 complex scalars (`cx`) with explicit construction,
  inspection, arithmetic and equality. Existing exact numbers and character
  literals keep their meaning.
- Add reproducible examples for exact systems, transformations, matrix
  factorization, least-squares fitting and complex rotations.

## Draft 0.21

- Build compact dense arrays directly from checked, non-escaping integer ranges.
- Extend packed traversal, intrinsic mapping and seedless axis reductions.
  Fuse selected total, same-shape real and Boolean expressions without
  changing observable order, rounding or logical work charges.
- Add paired construction controls and a numerical equivalence gate for
  optimized and generic typed execution. No Flux syntax changes.

## Draft 0.20

- Reorganize the single Rust crate by compiler, shared contracts, typed runtime,
  numerical arrays, native library, tooling and host interfaces. Isolate the
  historical oracle. Existing Flux programs and public Rust imports are unchanged.

## Draft 0.19

- Add Machina packed dense-array buffers, shared views and selected checked
  kernels, with exact fallback and no change to Flux syntax.
- Add validated host resource policies, an optimization-off typed reference
  mode, and paired checked benchmarks.

## Draft 0.18

- Add Mathesis: qualified `math` scalar and lifted functions for rounding,
  elementary and special functions, and exact integer mathematics.
- Add `poly` coefficient operations and bounded `num` root finding, integration,
  interpolation and differentiation with typed solver errors.

## Draft 0.17

- Add suspended `!^{ ... }` blocks with implicit unit and checked propagation
  from immediate unit-result execution statements.

## Draft 0.16

- Add explicit-arity tacit functions `&{N: ... }`, positional arguments,
  generalized forks and eager lexical captures. Unary tacit syntax is unchanged.

## Draft 0.15

- Add `form` and `dim`; extend shared numeric, Boolean, selection,
  mapping and reduction operators to dense arrays without changing sequences.
- Add axis permutation, sections, axis-specific folds and scans, and exact
  dot products, matrix multiplication and diagonals in `arr`.

## Draft 0.14

- Prefer `[:` (take), `:]` (drop), `sect` (section), `,.` (flatten one level),
  and `|:` (transpose). Earlier spellings remain compatibility aliases.
- Add `@/` for a seedless left reduction returning an optional.

## Draft 0.13

- Breaking syntax change: named type variables now use adjacent tilde
  prefixes (`~a`, `~row`) in types, row tails, generic declarations and
  constraints. Inference, character literals and tilde operators are unchanged.

## Draft 0.12.1

- Declare directly callable tacit functions, references and lambdas with
  adjacent `name/arity` headers. Ordinary function values still use `@`.

## Draft 0.12

- Add checked dense arrays, explicit broadcasting and rank mapping,
  last-axis scans, windows, indexed selection, and stable grade/permutation.

## Draft 0.11

- Add explicit structural row quantification and schema-preserving
  generic table transformations, including empty results.

## Draft 0.10

- Add parameterized recursive nominal sums and checked prefix matches
  over constructors, tuples, records, optionals, booleans and unit.

## Draft 0.9

- Give syntax occurrences stable identities within a checked program and
  validate typed HIR before execution. Tighten the value restriction and
  report numerical domain conflicts at their source.
- Consolidate the current language rules in one reference. Keep checked
  examples alongside it, not historical copies of the specification.

## Draft 0.8

- Format strict prefix source with hierarchical indentation, aligned
  margins, and token-preserving `flux fmt --check`.

## Draft 0.7

- Add typed tacit identity, composition, forks and hooks. Replace the
  public examples with a checked mathematics, science and data corpus.

## Draft 0.4 through 0.6

- Add constraint inference, checked HIR, typed errors, and local modules.
- Add arbitrary-precision exact numerics, decimal quantization, and
  schema-directed JSON and CSV. Add typed ordered joins and nullable
  table transformations.
