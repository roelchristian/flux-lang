# Flux Draft 0.5: Exact numerical foundations

This revision extends the typed-HIR numerical model. It leaves strict prefix
syntax, contextual numeric literals, explicit effects and the rest of the
language unchanged. It does not add decimal rounding contexts or implicit
conversion into approximate arithmetic.

## Domains and literals

| Type | Meaning |
|---|---|
| `n` | Arbitrary-precision nonnegative integer |
| `z` | Arbitrary-precision signed integer |
| `q` | Arbitrary-precision exact rational |
| `d` | Exact decimal: signed integer coefficient and nonnegative scale |
| `r` | Finite IEEE 754 binary64 value |

A rational is reduced and has a positive denominator. An integer does not
have a language-level overflow at the old 128-bit boundary. The reference
implementation can store small integers and rationals inline, then promote
or demote transparently without altering type or value. For example,
`+ 170141183460469231731687303715884105727z 1z` is the exact next integer.
Resource limits are not machine-integer overflow and may still produce
diagnostics. The reference runtime guards normalized numerator, denominator
and decimal coefficient magnitudes at 1,048,576 bits, and decimal scale at
10,000. These implementation limits do not round values or select another
numerical type.

An unconstrained integral literal defaults to `z`; an unsuffixed fractional
literal defaults to exact `q`. A suffix fixes the domain. A natural literal
must not be negative. `1.20d` is represented by coefficient 120 and scale 2,
denoting 120 × 10⁻². Its decimal spelling preserves the two fractional
digits even though it is numerically equal to `1.2d`. Decimal has no distinct
negative zero.

## Arithmetic and promotion

Within the exact domains the promotion order is `n`, `z`, `d`, `q`.
Generic type inference, direct calls and indirect calls must honor the same
checked domain. Approximate operands do not silently promote exact operands
to `r`; use an explicit conversion. Collection lifting preserves these
scalar rules and checks shapes dynamically where their lengths are not
statically fixed.

- Natural subtraction underflow is a fatal domain diagnostic.
- Exact `/` returns `q`, including with decimal operands. It never rounds a
  decimal quotient to a chosen scale.
- Integral `//` is floor division, and `% a b` is `a - b * floor(a/b)`. A
  nonzero remainder has the sign of the divisor. These operators retain
  the integral operand requirement.
- Ordinary division by zero, including in exact arithmetic, is fatal.
  Use `math.div` for an explicit `q^numerr` instead. It returns the wide exact
  quotient when valid; the existing `divzero` and `range` alternatives remain
  in the checked operation's type.

For a result in decimal domain, addition and subtraction retain the greater
operand scale, multiplication adds scales, and natural exponentiation
multiplies the scale by the exponent. A promoted integral operand enters
decimal domain at scale zero. Negation and absolute value preserve scale.
Exponent zero gives coefficient 1 and scale zero. Examples:

```flux
+ 1.20d 2.3d          -- 3.50d
- 1.20d 1.2d          -- 0.00d
* 1.20d 2.0d          -- 2.400d
^ 1.20d 2n           -- 1.4400d
/ 1.20d 3.0d         -- 2/5, of type q
```

Decimal equality, ordering and key identity are numerical, not string- or
scale-based. Scale still matters to display and interchange. An expression
promoted to rational domain does not carry display scale.

## Conversions and rounding

`:d` is an exact conversion and requires a finite decimal expansion. A
reduced rational can convert exactly if its denominator has no prime
factors other than 2 and 5. A nonterminating expansion is a fatal domain
diagnostic; it is not silently rounded. Re-converting a decimal preserves
its existing scale. For a finite `r` input, the stored binary64 number is
converted exactly, not reinterpreted from its shortest display.

`math.quantize SCALE VALUE`, from the explicit `math` import, converts an exact
value to decimal at the requested natural scale. It rounds to nearest, with
exact ties going to the even coefficient. The rule is symmetric for
negative values. For example, at scale two, `2.345d` rounds to `2.34d` and
`2.355d` rounds to `2.36d`. A rational such as `/ 1 3` can be quantized even
though its exact expansion cannot be represented as `d`. The returned
value carries the requested scale, including appended zeros. Resource
limits may reject unreasonable scale or arithmetic growth; they do not
select a lower precision.

`:q` on an `r` returns the exact rational value of the stored binary64
number, not the rational suggested by a decimal display. This applies to
finite subnormal values as well as ordinary values. Explicit conversion
to `r` rounds a single exact value to nearest binary64, ties to even, and
does not perform an intermediate approximate calculation. An overflowing
conversion reports a diagnostic rather than returning infinity.

## Interchange and execution

The REPL reports the static domain. Decimal display and typed CSV preserve
scale, including trailing fractional zeros. Typed CSV decodes malformed
numeric cells as structured CSV failures. A wide integer or rational is not
malformed merely because it exceeds the former inline representation. Raw
CSV remains a string operation. JSON, alternate decimal rounding modes,
and decimal monetary conventions are outside this revision.

The ordinary CLI, strict HIR mode, public Rust API and REPL share the typed
arithmetic implementation; the legacy oracle does not define new decimal or
wide-integer behavior.
