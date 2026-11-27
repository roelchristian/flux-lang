# Draft 0.4.2 implementation and review

The production CLI, REPL and public Rust check/run API now use a single typed
HIR. The prior checker and evaluator are compiled only for Rust tests or
the explicit `legacy-oracle` feature. The legacy `Policy::Compatible` API
spelling is an alias for typed preparation, not a way to select the oracle.

The registry supplies canonical identities, arities, availability and lifting
metadata to expression parsing, inference, lowering and evaluation.
Operation-specific constraint generation uses checked callable signatures.
The parser produces structured, source-ranged type annotations; only the
isolated legacy oracle consumes their normalized textual spelling.

## Independent verification and revisions

- The operation matrix accounts mechanically for every implemented and
  recognized-only registry entry. The five bundled executable examples
  pass under both CLI spellings, and the REPL and Rust API exercise the
  same typed fold implementation.
- A parity review found the first integration had reversed the maximum
  and minimum identities. The registry was corrected; a CLI regression
  checks the two signs. Independent probes then passed across fold/scan,
  static table evidence, curried numeric reference specialization, result
  callbacks, and failure ordering. Left-to-right operand evaluation and
  lazy branch selection were preserved.
- Ten existing differential specimens and 469 numeric conversion checks
  pass. A separate 33-case collection probe matched legacy outputs, and
  four negative probes preserved the failure reason and status. The
  documented sequence Cartesian product is also executable.
- Rust 1.85 formatting, full all-feature/all-target tests, Clippy with
  warnings denied and locked release build pass.

An unconstrained reference to a shape-overloaded builtin such as `&take`
may still require a contextual function type. Recognized but non-executable
operations such as joins, correlation, approximate equality and decimal
conversion have explicit compile-time diagnostics. Nothing falls back.

## Performance

Release builds, same Linux host pinned to the same CPU, nine interleaved
samples, medians including CLI startup.

| Workload | 0.4.1 | 0.4.2 |
| --- | ---: | ---: |
| 100,000-value numeric pipeline | 34.86 ms | 34.24 ms |
| Check 4,000 declarations | 29.23 ms | 29.12 ms |
| 6,000-row CSV aggregate, 32 keys | 8.68 ms | 8.18 ms |
| 6,000-row CSV aggregate, 6,000 keys | 21.28 ms | 21.49 ms |

These measurements cover the previously executable shared workloads, not
all newly migrated operations or every hardware configuration.
