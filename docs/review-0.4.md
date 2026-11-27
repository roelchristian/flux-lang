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
- Continue auditing evidence at higher-order boundaries. Typed calls now pass
  checked argument and result types through instantiation and partial
  application. This is not general overloading: the solver still rejects some
  ambiguous numeric-family and collection-shape combinations.
- Replace all textual type annotations in the parser with a canonical
  annotation AST. HIR uses canonical types, but the parser still stores
  tokenized annotation text for the frontend and compatibility checker.
- Expand structured diagnostics beyond general category codes and reliable
  primary spans to include related spans and typed evidence for every failure.
- Treat performance outliers as merge gates, not claims about the whole
  language. Compare strict typed workloads separately from compatibility runs.

## Independent review of PR #2

Separate reviewers checked numerical semantics, rank-one schemes/open rows,
dependency analysis, and the checker-to-HIR interface against PR head
`f21a291`. They reproduced the positive generic, open structural projection,
top-level recursion, and header-only grouped CSV cases using the actual CLI;
the CSV test deliberately made the summarizer fail if ever executed.

### Corrected after review

- Exact division now handles zero over the minimum bounded integer before
  cancellation. Explicit conversions between bounded rationals and binary64
  round correctly or report a numeric bound; finite reals can convert back
  exactly when their fraction fits.
- Reject nested elementwise comparison/logic, nonscalar range bounds,
  unsupported decimal CSV fields and unsupported ordering at check time.
- Explicit open-row annotation syntax is rejected pending kinding and
  rigidity rules. Inferred private open rows are still supported.
- Tuple spread applies already-evaluated operands one at a time, including
  when saturation returns another function.
- The CSV no-clobber test uses a Windows-valid temporary directory name.

### Follow-up fixes and release gates

- **Local initialization:** the resolver rejects eager reads reached by direct,
  mutually recursive and statically tracked callable aliases, including
  known tuple and record projections, while allowing
  closure creation, latent capture and safe forward calls. This is not a
  theorem about arbitrary higher-order calls: a callable that escapes the
  static analysis still needs the runtime initialization guard.
- **Scoped nominal identity:** the typed frontend preserves a schema's
  resolved identity independently of its display name. Nested declarations
  are compile-time only in HIR; two same-spelling schemas are not
  interchangeable. Compatibility execution has not been upgraded to all
  typed scoping rules.
- **Signature rigidity and empty ambiguity:** an explicit type variable
  cannot specialize to a concrete type or escape into a captured monotype.
  Unconstrained empty sequences are rejected even inside larger expressions;
  contextually typed empties and explicitly polymorphic empty sequences
  remain legal. Explicit open-row annotation syntax stays out of scope.
- **Checked calls:** typed argument and result information is preserved across
  supported user-function and builtin instantiations, including escaping
  closures and indirect partial applications. HIR stores ordered schema
  metadata for table-producing operations independently of row data.
- **Large-file checking:** contextual literal metavariables were the largest
  measured cost on the 4,000-declaration fixture. An unannotated closed scalar
  integer expression can take the language's default `z` without allocating
  those variables. References, contextual annotations, lifted shapes and
  other operations still use the solver. The checker still builds resolved
  metadata and validates HIR. The renewed benchmark below shows remaining cost.

### Follow-up verification

The independent regressions are in `tests/review_fixes.rs`. They use
`check` for static acceptance or rejection and `run-hir` for executable
controls, not compatibility fallback. They pair failing cases with legal
forward calls and latent captures, same-spelling nominal declarations in
separate lexical scopes, a locally polymorphic explicit identity, and
contextually typed empty sequences. A schema assertion inspects the
aggregate HIR node; the executable counterpart writes a header-only CSV
when the aggregate is reached through a function and a local summarizer
alias. Numeric runtime tests exercise an escaping constrained closure
and a partially applied built-in reference. Those runtime tests alone
cannot establish that an evaluator never reconstructs evidence from
runtime value tags.


### Recheck after the implementation fixes

Nine paired release runs on the same Linux host, Rust 1.85. These are
end-to-end medians, not the separate per-phase timings.

| Workload | Draft 0.3 (ms) | Review fixes (ms) |
|---|---:|---:|
| numeric pipeline (100,000 values) | 39.49 | 34.45 |
| lex/parse/check (4,000 declarations) | 11.61 | 27.65 |
| typed CSV + aggregate (6,000 rows, 32 keys) | 11.40 | 6.53 |
| typed CSV + aggregate (6,000 rows, 6,000 keys) | 31.72 | 20.73 |

The 4,000-declaration check fell from roughly 53 ms before the fixes to 28 ms,
but remains about 2.4x slower than Draft 0.3. On the 4,000-declaration phase
fixture, parsing was 6.9 ms, resolution 5.5 ms, inference 5.7 ms, and validated
HIR elaboration 7.5 ms. The residual cost is still a decision to review before
merging; it is not removed by bypassing HIR validation.

Linux formatting, all 123 tests, Clippy, and a locked release build passed.
The enhanced differential harness compares static check status and executes
Draft 0.4 in strict typed mode; all ten common specimens matched Draft 0.3.
The independent numeric oracle checked 469 bounded rational/binary64
conversions against Python's Fraction.
