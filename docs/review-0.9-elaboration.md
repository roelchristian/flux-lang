# Draft 0.9: independent review and verification

Baseline: Draft 0.8, `3e36302`. The independent reviewer did not write the
implementation. They reproduced failures through the public Rust entry points
and reran the revised behavior separately from the implementation workstreams.

| Finding | Revision and verification |
| --- | --- |
| Inference panicked when the AST was an exact clone but resolver metadata came from the original. | Public passes rebind stable occurrence IDs to exact trees; foreign semantic trees get structured errors. A matching reparse refreshes expression spans. |
| HIR admitted a load naming another closure's formal and a table with no closed schema witness. | Admission verifies lexical ownership and required concrete schema evidence. Mutated HIR is rejected before evaluation. |
| An inner raise could target an enclosing boundary; the result changed without a type error. | Admission verifies the nearest boundary and respects the closure and suspended-action barriers. |
| Removing arithmetic evidence still allowed the call to execute. | Required arithmetic evidence is checked, including in a dormant branch. Generic checked calls still work. |
| An expansive right-hand side could silently specialize a universally annotated binding. | A declaration that violates the value restriction now fails, including an otherwise unused module export. |

## Verified on Linux with Rust 1.85

- Formatting, locked all-features/all-targets test suite (323 tests), strict
  Clippy and locked release build passed.
- The corpus test checks all 52 executable examples, including output and
  generated files, through normal and strict typed execution.
- All ten typed differential specimens matched the Draft 0.8 binary. All
  1,020 independently calculated exact numerical specimens matched.
- Module interfaces, three syntax-identity tests, seven inference tests and
  six HIR validation tests are covered by the Rust suite.
- Cross-platform GitHub CI is required before merging; it was not run locally.

## Performance

One alternating 17-pair release run on the same Linux machine, elapsed
median in milliseconds (startup included):

| Workload | 0.8 | 0.9 |
| --- | ---: | ---: |
| Numeric pipeline, 100,000 elements | 70.946 | 69.135 |
| Static check, 4,000 declarations | 50.730 | 67.497 |
| Typed CSV and aggregation, 6,000 rows / 32 keys | 13.643 | 11.801 |
| Typed CSV and aggregation, 6,000 rows / 6,000 keys | 30.971 | 30.011 |

The static check is measurably slower in this sample. An alternating phase
sample attributes the additional work to resolution and elaboration
(including the new validation pass), not constraint inference. The ordinary
compiler path does not copy or structurally compare its AST between these
phases; that check is reserved for public callers reusing separate pass
results. Timing variability is substantial on this machine. Treat the
measurement as a review point, not a promise of relative performance.

## Remaining scope

IDs are stable within one semantic program shape, not across arbitrary edits.
The checked runtime still fuses implicit exact-operand conversions with
numeric kernels. This is not a global constraint-solver rewrite, and local
forward references retain the generalization restriction described in the
Draft 0.9 notes. No grammar or new language feature is introduced here.
