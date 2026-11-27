# Draft 0.7: independent review and verification

The final Draft 0.6 baseline is PR #8 as merged, including its independent
review fixes. The tacit parser lowers entirely to existing typed function
references, closures and applications. The lexer/parser review checked
maximal munch, module aliases, local shadowing, bounded nesting, generated
binder hygiene and diagnostic ranges. It caught unintended acceptance of
ampersand-prefixed atoms inside the block; the final grammar requires bare
atoms, with explicit identity reference available outside. Missing or extra
operands now report the combinator's fixed arity or the additional operand.
Review also caught a backward-compatibility break: compact ordinary
application to a selector or guarded expression was split as a tacit sign.
Existing complete operand tokens now retain their boundaries, covered by
ordinary and strict-HIR CLI regressions.

A separate runtime review tested exact overloads, structural and nominal
selectors, currying, polymorphic imported values, action construction,
recoverable outcomes, strict HIR and formatter/REPL behavior. The right branch
of a fork is evaluated even with a Boolean combiner whose ordinary direct
call might short-circuit. The integration suite compares tacit and ordinary
definitions and exercises the synthesis of keys and exact values in the
synthetic reconciliation. No new execution or lookup rules are introduced.

## Performance

Seven alternating release-mode pairs on one Linux host (medians, ms).
Both forms gave exactly the same output in each runtime workload.

| Workload | Explicit | Tacit | Tacit / explicit |
| --- | ---: | ---: | ---: |
| Repeated scalar composition | 35.730 | 34.705 | 0.971 |
| Square, map and reduce | 23.191 | 23.252 | 1.003 |
| Financial column projection | 54.699 | 53.420 | 0.977 |
| Join with transformed keys | 126.613 | 126.952 | 1.003 |
| Nested composition, static check | 1.993 | 1.885 | 0.946 |

Reproduce with `python3 benchmarks/tacit.py target/release/flux`.
These are end-to-end medians, not isolated allocation timings and not
guarantees for arbitrary program sizes. No special runtime optimization
was needed. Rank polymorphism and limits of the existing general-purpose
type solver are unchanged. The parser bounds tacit nesting at 64 levels to
keep lowered expression depth within normal frontend limits.
