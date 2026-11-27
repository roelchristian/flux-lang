# Draft 0.4.1 implementation and review

Baseline: merged Draft 0.4, `e3136d7`. Typed errors extend the canonical
checker and HIR rather than adding an interpreter inside the evaluator.
`run-hir` remains the strict entry point. New syntax and migrated I/O cannot
use the Draft 0.3 compatibility evaluator.

## Independent review

The independent review exercised generic and nested Result values, lexical
shadowing of same-spelled nominal constructors, recursive and nested handler
boundaries, provenance-independent equality, selected recovery arms, and
I/O order (including a closed output pipe). It found that an unused pipeline
stage could silently discard a Result. The checker now rejects that case
using the resolver's identity for the next stage's `$`, while allowing explicit
propagation and recovery. Integrated conformance also caught two lowering
gaps: table evidence did not follow the success type of a Result, and HIR
looked up CSV aliases by surface spelling. Both are now covered by strict
execution tests. The final user specimen uses a unit tail so it does not
discard a Result or return an optional exit status.

Validation on Rust 1.85: all 161 tests, formatting, Clippy (warnings denied),
and locked release build. Ten Draft 0.4 differential specimens preserve
stdout, exit status and output-file bytes after the explicit I/O migration;
469 exact/approximate conversion cases match the numeric oracle.

## Performance

Release builds, same Linux host and CPU affinity, 15 interleaved samples per
binary. Medians include CLI startup and, where applicable, equivalent source
using the new explicit I/O handling.

| Workload | Draft 0.4 | Draft 0.4.1 |
|---|---:|---:|
| 100,000-value numeric pipeline | 31.94 ms | 33.01 ms |
| Check 4,000 arithmetic declarations | 27.78 ms | 28.42 ms |
| 6,000 CSV rows, 32 keys | 5.86 ms | 6.42 ms |
| 6,000 CSV rows, 6,000 keys | 19.43 ms | 19.76 ms |

The first integration put error and sum payloads inline. Even holding failure
provenance by reference was not enough: with inline nominal variants, Rust
1.85 widened a runtime value slot from 48 to 64 bytes. This inflated ordinary
numeric sequences too. Keeping both failure provenance and variant payloads
behind shared pointers recovered most of the pipeline regression. Residual
low-cardinality CSV overhead is reported above rather than described as
parity. The new 10,000-element mixed recovery workload took about 22.8 ms
in the evaluation phase in the earlier compact-provenance build. The harness
selects legacy fixtures with `--legacy-before` when comparing pre-0.4.1
binaries.

## Remaining boundaries

- Typed recovery patterns support one constructor with immediate payload
  binders (or a catch-all); this does not introduce general pattern matching.
- Checked numerical operations use bounded exact arithmetic.
  `math.div` reports a typed range failure rather than adding arbitrary
  precision. There is no typed alternative for every partial operation yet.
- The host reader retains UTF-8, regular-file and byte-limit checks. Tests
  can inject input streams and an ordered output writer; writes and existence
  checks use the host filesystem. There is no rollback or atomic replacement.
- Exceptions do not provide a new reason to select compatibility mode.
  Some unrelated library operations still lack typed lowering in Draft 0.4.
- The Result collection helpers and user-defined resource-management
  facilities are separate workstreams.
