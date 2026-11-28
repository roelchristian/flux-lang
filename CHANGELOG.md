# Flux changelog

The [language reference](docs/specification.md) describes current behavior.
This log summarizes material changes. Earlier text and design revisions
are available through [the repository's commit history](https://github.com/roelchristian/flux-lang/commits/main/);
obsolete specifications are not kept as duplicate files.

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
