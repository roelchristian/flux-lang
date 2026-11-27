# Draft 0.4.2: Typed HIR consolidation

Baseline: Draft 0.4.1, main at `eaa5ff8`. This plan consolidates the language
already specified rather than adding new syntax or library capabilities.

## Required architecture

1. Make the builtin registry the authoritative source for operation identities,
   spellings, aliases, parse arity and runtime support status. A spelling or
   alias has one semantic identity. Signature/constraint generation can use
   operation-specific rules, but must dispatch through that identity. A
   recognized non-executable operation gets an explicit compile-time
   diagnostic; it cannot be accepted by a fallback interpreter.
2. Parse type annotations into canonical syntax nodes instead of passing text
   to another parser. Retain annotation source locations, precedence and
   nominal names; resolve the names lexically, separate from syntax parsing.
   Preserve public annotation behavior including qualified polymorphism,
   record/table schemas, result/action nesting and valid parenthesization.
3. Lower every existing executable operation to checked HIR. Resolve callable
   arity, instantiations, collection/row shape, operation/lifting mode and
   relevant numeric domains before execution. Transport evidence across
   direct, indirect, partial, spread and higher-order calls, including
   callbacks passed to collection operators. Checks on dynamic bounds,
   lengths and failure conditions remain at runtime; the evaluator must not
   use runtime tags to choose a different static overload.
4. Route CLI, REPL and public Rust execution through the same typed HIR.
   Unsupported operations must be distinguishable from type errors, invariant
   violations and recoverable failures. Keep a legacy interpreter only as an
   optional differential oracle, never as a production recovery path.

## Workstreams

- Inventory all runtime registry entries; identify existing HIR support,
  missing operations, aliases, context-dependent forms, and reserved syntax.
- Port fold and scan (`/:`, `\:`), collection concatenation and lookup
  (`+:`, `.:`), distinct and zip (`=.`, `&:`), Cartesian product and slicing
  (`*:`, `[:`, `take`, `drop`, `slice`), reversal and shape transformations
  (`rev`, `flat`, `tr`), and statistical and boolean reductions
  (`var`, `pct`, `>/`, `</`, `&&/`, `||/`).
- Migrate existing examples and public entry points without weakening type
  checking, result handling, explicit action execution, or CSV schema
  transport.
- Use the baseline differential oracle where its behavior is intentional;
  do not copy an implementation accident or let outdated behavior override
  the typed specification.

## Acceptance gates

- Every registered **runtime** operation has typed implementation and tests;
  registry-to-HIR coverage is mechanically checked, with concrete public
  CLI specimens to verify semantics. Aliases use the same operation,
  constraints, and evidence as the canonical spelling.
- Every bundled executable example runs in typed HIR with expected output.
  Check/running the same source by ordinary CLI and strict HIR selection
  must not change the accepted language or introduce interpreter fallback.
- Cover reductions and accumulators, empty and invalid inputs, order and
  shape preservation, exact arithmetic, lexical closures, indirect calls,
  discarded results and error provenance. Reserved joins (`*=`, `*?`)
  and unsupported numeric types remain explicit compile-time boundaries.
- Run formatting, Rust 1.85 tests, Clippy with warnings denied, release
  build, covered differential workloads and reproducible same-host
  performance comparisons. Record commands and results separately from
  prospective gates; a green coverage test does not prove deployment,
  performance parity or feature completion outside this scope.

Source modules and interfaces are the separate Draft 0.4.3 P0. Arbitrary
precision, runtime decimals, general matching, a prelude, relational joins,
columnar execution and language syntax expansion are outside this revision.

## Conformance inventory

`tests/draft042.rs` requires its canonical operation inventory to equal the
registry's executable and recognized-only sets. It runs each executable
specimen through the public strict CLI, checks explicit rejection of each
recognized-only entry, and runs all bundled scripts through strict and
ordinary execution in an isolated directory, checking their stdout and
generated CSV. Separate cases exercise exported aliases, typed I/O effects,
empty collection shapes, runtime domain diagnostics, and evidence across
higher-order functions. The same fold is checked through the public Rust
API and evaluated interactively through the REPL.
