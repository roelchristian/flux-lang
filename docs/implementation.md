# Flux implementation guide

The [language reference](specification.md) defines current behavior. This
guide describes the Rust reference implementation. See the [changelog](../CHANGELOG.md)
for notable changes and use Git for earlier revisions.

## Compiler and runtime

1. `src/lexer.rs` validates ASCII source and records byte spans. Parsing
   discovers declaration and imported callable arity before it consumes
   prefix operands. Arity is lexical, not inferred from a value's type.
2. `src/modules.rs` loads local source modules and checks interfaces.
   `src/resolve.rs` assigns lexical symbol identities and dependency groups.
3. `src/infer.rs` and `src/types.rs` check rank-one schemes, shape and member
   constraints, and exact numeric joins. Syntax IDs align expression types
   and operations with their source occurrences throughout the compiler.
4. `src/hir.rs` elaborates typed calls, lift modes, table schemas, closures
   and error scopes. Admission checks HIR before it reaches the evaluator.
   The compiler keeps one borrowed tree; separately exposed passes also
   verify that externally reused metadata describes the same syntax.
   Reuse requires the same declaration and pattern offsets; edits that
   move them require fresh resolution even when expression IDs agree.
5. `src/hir_eval.rs` executes checked HIR. It uses bounded-resource
   arbitrary-precision exact arithmetic, exact scaled decimals and
   explicit approximate operations. Runtime I/O is attached to suspended
   actions. Neither script execution nor the REPL falls back to the old
   evaluator; the `legacy-oracle` Cargo feature is for regression work.
6. `src/formatter.rs` uses the original token stream and opt-in parser
   ranges to format the requested source without altering its semantics.

## Verification

Use Rust 1.85 or later. From the project root:

```sh
cargo fmt --all -- --check
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo build --locked --release
```

The example corpus checks every registered program's stdout, exit status and
declared files through both normal and strict HIR execution. Use
`cargo test --locked --test example_corpus` to run it alone. For current
release binaries, `benchmarks/differential.py --typed-oracle BEFORE AFTER`
compares a supported subset independently of the legacy interpreter.
`benchmarks/exact_numeric_oracle.py` uses independently calculated integer,
rational and decimal values. Report measurements with workload, compiler and
mode; see the [performance guide](performance.md) for benchmark commands.

## Limits

Programs are checked and evaluated eagerly; selected representations and
numeric operations have resource limits. JSON decoding is schema-directed,
with limits on depth, size and exact-number expansion. Source modules are
local and explicitly checked, not packages. Relational execution preserves
specified ordering; it is not a columnar query optimizer. Proposals are not normative until implemented and tested.
