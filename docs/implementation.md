# Flux implementation guide

The [language reference](specification.md) defines current behavior; this
guide explains the Rust pipeline and verification.

## Compiler and runtime

| Stage | Contract |
| --- | --- |
| Lexer, parser and modules | Record byte spans. Discover local and imported callable arities before parsing prefix operands. Lower `name/arity` to ordinary named functions with hygienic synthetic inputs. |
| Resolution and inference | Assign lexical symbols and dependency groups. Check rank-one schemes, constraints, shapes and exact numeric joins. Stable syntax IDs associate expression types and selected operations with source occurrences. |
| Elaboration and admission | Lower checked calls, lifts, schemas, closures and error scopes to typed HIR. The compiler shares one borrowed syntax tree. Independently reused metadata must describe the same syntax, including declaration and pattern offsets; matching expression IDs alone is not sufficient. |
| Evaluation | Execute only admitted HIR in scripts and the REPL. The `legacy-oracle` Cargo feature enables a separate differential-testing interpreter, not fallback. Runtime I/O is attached to suspended actions. |
| Formatting | Use original tokens and opt-in parser ranges to change layout without changing the token stream. |

Evaluation is eager and resource-bounded. Dense-array HIR selects the
same numeric operation for direct or first-class application and carries
the specific lift for scalar or pairwise broadcasting. The runtime
checks dynamic shapes and walks row-major offsets rather than copying
expanded singleton data. Numeric, JSON and array limits are in the
[reference](specification.md); relational evaluation is ordered,
not a columnar optimizer.

## Verification

Use Rust 1.85 or later. From the project root:

```sh
cargo fmt --all -- --check
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo build --locked --release
```

Integration tests are grouped into `compiler`, `language`, `data`,
`tooling` and `examples` targets (`cargo test --locked --test NAME`).
The [example harness](../examples/README.md#inputs-and-verification)
runs ordinary and strict HIR paths. Compare two release binaries with
`python3 benchmarks/differential.py BEFORE AFTER --typed-oracle`;
[the exact numeric oracle](../benchmarks/exact_numeric_oracle.py) supplies
independent integer, rational and decimal expectations.

## Performance

Use locked release builds and record workload size, compiler, platform
and mode. Retain the old binary for paired comparisons.

| Harness | Workload |
| --- | --- |
| [compare.py](../benchmarks/compare.py) | Alternate binaries on script, checking and grouped CSV workloads |
| [phases.py](../benchmarks/phases.py) | Parse, resolve, infer, elaborate and evaluate separately |
| [tacit.py](../benchmarks/tacit.py), [relational.py](../benchmarks/relational.py) | Composition and relational workloads |

Measurements are environment-specific; see the
[Draft 0.9 comparison](https://github.com/roelchristian/flux-lang/pull/13)
for an example. Earlier reports are in Git history.
