# Draft 0.4 migration and performance review

Baseline: Draft 0.3, commit `e9800db`. This branch leaves the original
evaluator and checker available for differential conformance and compatibility.
It does not claim that the whole language uses one evaluator yet.

## What is exercised through HIR

- Independently instantiated rank-one schemes, inferred lambdas, higher-order
  composition, direct/explicit/spread calls, and constrained named signatures.
- Lexical identities and top-level SCC ordering. Eager value cycles are
  rejected. Local functions preserve the environment at their declaration.
- Structural row projection; nominal schemas remain distinct. Empty tables and
  grouped results carry static field metadata through execution and CSV output.
- Checked exact numeric operations, explicit effects, strict pipeline stage
  bindings, bounded CSV ingestion, and a no-clobber CSV writer.

`flux hir FILE` inspects elaboration. `flux run-hir FILE` executes only the
typed backend. An `E0400` is a migration boundary, not a type error. Ordinary
CLI execution falls back only when a boundary is explicitly reported and
the existing checker validates the program. It never falls back from a new
resolution error, type error, or invalid HIR.

## Checks

Run with Rust 1.85:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
python3 benchmarks/differential.py /path/to/e9800db/flux target/release/flux
python3 benchmarks/compare.py /path/to/e9800db/flux target/release/flux
python3 benchmarks/phases.py target/release/flux
```

The differential harness compares process status, stdout, and generated CSV
bytes across shared specimens. It does not assume the new diagnostics have
the same wording as the old ones. The phase harness requires typed-only
lowering and reports lexer/parser together, then resolution, inference,
elaboration and evaluation separately. Evaluation timing includes stdout
output and is effectively empty for declaration-only input.

## Measured results

Seven paired runs, Rust 1.85 release, same Linux x86-64 host. All ten
differential specimens matched; all 98 tests passed. Formatting, Clippy
with warnings denied, and release build passed locally.

| Workload | Draft 0.3 (ms) | Draft 0.4 (ms) |
|---|---:|---:|
| numeric pipeline (100,000 values) | 38.64 | 32.07 |
| lex/parse/check (4,000 declarations) | 11.17 | 53.12 |
| typed CSV + aggregate (6,000 rows, 32 keys) | 11.37 | 5.86 |
| typed CSV + aggregate (6,000 rows, 6,000 keys) | 30.88 | 14.51 |

The 4,000-declaration check is a **merge blocker pending profiling and review**.
The two compatibility-level executables do different amounts of analysis:
Draft 0.4 records source spans, builds lexical dependencies, solves each
binding and constructs typed metadata. This is not a claim that the observed
4.8x regression is unavoidable. Static checking could stop after validated
inference when HIR materialization is not requested, and source/type
metadata construction needs profiling. Large check and execution performance
are separate gates.

Five phase runs put inference for 1,000 independent declarations at 1.86 ms
and 1,000 polymorphic applications at 3.40 ms. The same workload before the
transactional-copy removal independently measured about 105 ms and 166 ms,
respectively (three release runs); this comparison diagnoses the internal
cloning bottleneck, not improvement over Draft 0.3. A 10,000-value range script
spent 0.93 ms in evaluation; declaration-only cases skip execution.

## Remaining migration gates

- Eliminate compatibility lookup/check/evaluation for remaining library
  operations. Keep the oracle until parity is established. The registry owns
  names, arities and numeric promotion; not every rule is a declarative
  constraint generator yet.
- Carry concrete evidence through indirect uses of overloaded built-ins and
  generic numeric bodies. The current solver rejects some numeric-family and
  collection-shape ambiguities rather than attempting arbitrary promotion.
- Replace all textual type annotations in the parser with a canonical
  annotation AST. HIR uses canonical types, but the parser still stores
  tokenized annotation text for the frontend and compatibility checker.
- Expand structured diagnostics beyond general category codes and reliable
  primary spans to include related spans and typed evidence for every failure.
- Treat performance outliers as merge gates, not claims about the whole
  language. Compare strict typed workloads separately from compatibility runs.
