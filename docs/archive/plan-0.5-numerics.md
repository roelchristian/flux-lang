# Flux P1: exact numerical foundations

Base: the checked source modules branch (PR #5). Preserve fixed arity,
signed-integer default, numeric promotion, and the exact/approximate split.
Do not introduce new numeric punctuation or legacy execution.

1. Keep small natural, integer and rational values inline. On machine
   overflow, promote to arbitrary-precision signed integers or normalized
   rationals and demote where possible. Natural underflow and division by
   zero are still domain errors. Use the Num crates for wide arithmetic.
   Resource limits remain separate from fixed-width numerical semantics.
2. Implement exact `d` values with signed coefficient and scale. Preserve
   literal scale; + and - retain the greater operand scale, multiplication
   adds scales, and exponentiation multiplies scale by its natural exponent.
   Ordinary exact division returns q (even for decimal operands).
   Explicit `:d` requires a finite decimal expansion. Provide qualified
   `math.quantize SCALE VALUE` for explicit half-to-even rounding of exact
   values to a requested scale. Decimal equality is numerical, not textual.
3. Centralize typed domains, promotions and checked arithmetic in HIR.
   Preserve exact finite binary64 conversions (including subnormals) and
   correctly rounded exact-to-binary64 conversion, with overflow diagnosed.
   Carry new numerics through collections, statistics, CSV and the REPL.
4. Independently compare wide exact arithmetic and rounding to an oracle,
   test literal/CSV scale and mixed domains across normal and strict CLI,
   and review performance on unchanged small workloads before opening a
   separate PR on the module branch.
