# P1 numerical foundations: implementation and independent review

Base: the local-source-module branch (PR #5). The typed evaluator extends
rather than changes the ordinary arithmetic model. Small exact values retain
the inline fast path; overflow of that representation is retried with
normalized arbitrary precision, and values are demoted when possible.
Decimal coefficient and scale survive representation, arithmetic and typed
CSV. Division of exact operands is rational; finite `:d` and explicit
half-even `math.quantize` are distinct operations.

The implementation uses locked Num crates; Rust 1.85 runs the existing and
new tests. The independent suite matched 1,020 generated integer, rational,
decimal and binary64 cases plus all 469 previous conversion cases. A second
review ran separate Fraction/Decimal/CSV fixtures and 175 wide binary64
round-trip checks, including signed subnormals and overflow thresholds.
Runtime budgets constrain capacity, not the precision at which arithmetic
is performed. A budget violation never silently truncates or rounds.

## Release medians

Same pinned Linux CPU, nine interleaved runs, including CLI startup.

| Shared workload | Source-module base | Wide numerics |
| --- | ---: | ---: |
| Numeric pipeline, 100,000 values | 32.37 ms | 32.49 ms |
| Check 4,000 declarations | 33.74 ms | 31.58 ms |
| Typed CSV aggregate, 32 keys | 6.73 ms | 6.73 ms |
| Typed CSV aggregate, 6,000 keys | 19.87 ms | 19.88 ms |

These measurements do not imply constant cost for wide values. The exact
arithmetic and decimal scale limits are documented in the specification.
