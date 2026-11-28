# Flux Language Specification: Draft 0.4

## Scope and status

Draft 0.4 changes semantic analysis, not Flux's general syntax. Draft 0.3's strict prefix notation, fixed syntactic arity, ASCII source, lowercase identifiers, immutable bindings, anonymous script entry and explicit actions remain the surface rules. This document distinguishes required semantics from migration coverage. In this revision, `d` in a type representation does not imply executable decimal arithmetic.

## Frontend and checked program

The semantic pipeline is source → lexer → declaration and interface discovery → arity-aware parser → untyped AST → lexical name resolution → inference and constraint solving → semantic elaboration → typed HIR → reference execution. A parser decides where operands end before inference. Resolved names identify declarations and binders rather than repeatedly looking up their text; compiler-generated symbols have their own identities. Parsing, checking, and evaluation must not silently reinterpret a value binding as an arity-bearing named callable.

Canonical types distinguish `n`, `z`, `q`, `d`, `r`, `b`, `s`, `c`, unit, sequences, tuples, dictionaries, optionals, suspended actions, curried functions, tables, nominal types and structural records. Variables stand for unknowns, not dynamic values. Type schemes quantify variables and carry predicates; each use of a generalized declaration gets fresh variables. Monotypes remain shared where required inside recursive inference. Declared parameters and results are checked, not treated as casts. Metadata uses canonical types and record/schema identity rather than re-parsing printed type strings.

## Dependency order and inference

Resolve lexical identities, then construct dependencies from resolved uses. Infer each strongly connected component as a group. Acyclic dependencies may precede their textual references when the existing scope permits it; ordinary local values still enter scope after their declarations. Initialize a recursive callable group with monomorphic placeholders; infer its bodies, solve the group, and only then generalize eligible declarations. Cyclic eager value initialization is an error. Do not repeatedly check unrelated declaration bodies until one happens to succeed.

Unify compatible type constructors recursively, with an occurs check to reject infinite types. Generalize eligible variables not free in the enclosing environment; respect the value restriction and keep predicates that belong to generalized variables. Resolve constraints using operand type, result context and registered signatures. An unresolved constraint that is eligible for quantification is not an arbitrary default; an executable use with no determinate implementation is an ambiguity. For example, `empty := [];` alone must not silently choose an element type. Error diagnostics should point to the smallest reliably known source span and retain the involved types or constraint.

```flux
id(x) := x;
cp(f,g,x) := @ f (@ g x);
identity := \x -> x;
sq(x:'a):'a | num 'a := * x x;
a := id 12;
b := id "twelve";
```

An unsuffixed integral literal follows an appropriate expected numeric type or defaults to `z`; an unsuffixed fraction defaults to exact `q`. An explicit suffix is a type choice. Permitted exact conversions/lifts must be deterministic and inserted deliberately during elaboration. Mixing exact and approximate arithmetic without a permitted conversion is not resolved by silently making both approximate. Division and underflow behavior remain the Draft 0.3 behavior. Capability predicates, such as numeric operations and equality, constrain type uses independently of unification. Functions and actions have no structural equality.

## Structural rows and table schema

A structural field requirement may be inferred from projection. For example, `amount(x) := .amount x;` requires the field `amount` but need not require the rest of the structural row. Each call can instantiate both the field type and the remaining row, while a declared nominal record remains distinct. A missing field or a wrong type for a required field is an error. Repeated fields are not legal. Inferred constraints may be reported as an open row with a tail variable; the row variable denotes only the remaining fields and cannot repeat an explicitly named field. Explicit open-row annotations are not part of the source language in this draft.

```flux
amount(x) := .amount x;
balance := amount .{ amount := 12; label := "local"; };
ratio := amount .{ valid := 1b; amount := 0.5; };
```

Field lookup must not erase field names, and field presentation order is the declared/construction order for schemas, including CSV output. An empty table still has its checked row schema. `#:` from equisized columns preserves each field's element type even when all columns have length zero. Grouped aggregation produces a checked result schema from the summarizer's result type and never invokes it to invent a row when there are no groups. Projections, extensions, slices and empty writes use that schema rather than attempting to inspect an absent example row or a function body. Operations that combine schemas must retain the specified stable field and row ordering.

## Elaboration, effects and pipeline invariants

A checked HIR operation carries its resolved semantics, relevant type and source span. It should encode explicit conversions, operation identity or overload, function/symbol references, effect actions, schema-dependent table operations and ordered control flow. The reference evaluator consumes the elaborated operation: it may inspect data for runtime domain/shape checks but should not re-infer a static schema or re-resolve overloads. In particular, the source parser cannot recover a missing operand during checking, and the evaluator should not infer a different overload merely because a value happens to be integral at runtime.

Constructing an action never executes it. A `!` remains necessary at an effect-legal position. Pipelines bind the preceding stage immutably, evaluate each stage once and preserve enclosing placeholder scope under nesting. A function-value application remains explicit: `@ f (x,y)` supplies a single tuple and `@@ f (x,y)` supplies two curried operands.

## Implementation and migration boundary

The checked HIR core, the production CLI dispatch and compatibility execution must be assessed independently. A successful old example does not prove it ran through HIR. A typed registry/interface summary is only authoritative for call sites that actually consume it. A compatibility path, where retained, must be identified rather than presented as fully migrated. The conformance tests exercise the executable's public `check` and run modes; internal solver/HIR tests separately exercise their representations. A test passing on the compatibility evaluator does not prove HIR schema transport.

Runtime decimal arithmetic, arbitrary precision, general exception syntax, package resolution, tensors, columnar execution and expression fusion are outside this revision. The reference subset uses bounded arithmetic and supported finite approximate operations; runtime value bounds are not redesigned into Flux's general mathematical semantics.

### Current migration API

`semantic::prepare` makes the execution boundary observable. `Policy::TypedOnly` either returns a checked `HirProgram` or reports a migration error. `Policy::Compatible`, used by the ordinary CLI, may select the explicitly identified Draft 0.3 path for an unsupported migration boundary. A resolution or type error in the new path is not grounds to retry through the old checker. `Prepared::hir` and `Prepared::migration_reason` identify which path was selected.

Use `flux hir FILE` to inspect typed lowering and `flux run-hir FILE` to require
typed execution. Both reject an explicit migration boundary. The typed
evaluator covers rank-one closures, direct scalar and lifted arithmetic,
strict pipelines, structural and nominal values, schema-carrying tables,
filtering and grouping, and raw and typed CSV. Empty aggregation is tested
through typed execution even when its summarizer calls a helper. See the
[migration and performance review](review-0.4.md) for remaining backend,
generic evidence and diagnostic limitations.
