#!/usr/bin/env python3
"""Cross-check Mathesis against independently calculated values.

Test exact output with Python integers and fractions, and finite approximate
output with the standard math library and analytic polynomial formulas.
"""
import argparse
from dataclasses import dataclass
from fractions import Fraction
import math
from pathlib import Path
from random import Random
import subprocess
from tempfile import TemporaryDirectory


@dataclass(frozen=True)
class Case:
    expression: str
    expected: object
    real: bool = False
    result: bool = False


def q(x):
    return f"(/ {x.numerator}q {x.denominator}q)"


def r(x):
    return f"{float(x)!r}r"


def seq(items):
    return "[" + " ".join(map(str, items)) + "]"


def factors(n):
    candidates = []
    k = 2
    while k * k <= n:
        while n % k == 0:
            candidates.append(k)
            n //= k
        k += 1
    if n > 1:
        candidates.append(n)
    return candidates


def fib(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a


def exact_cases(rng):
    x = Fraction(rng.randrange(-(1 << 200), 1 << 200), rng.randrange(1, 1 << 52))
    for name, y in [
        ("floor", math.floor(x)), ("ceil", math.ceil(x)), ("trunc", int(x)),
        ("round", round(x)), ("sign", (x > 0) - (x < 0)), ("frac", x - int(x)),
    ]:
        yield Case(f"math.{name} {q(x)}", y)
    if x:
        yield Case(f"math.recip {q(x)}", 1 / x)
    a, b = [rng.randrange(-(1 << 197), 1 << 197) for _ in range(2)]
    yield Case(f"math.gcd {a}z {b}z", math.gcd(a, b))
    yield Case(f"math.lcm {a}z {b}z", math.lcm(a, b))
    base, exp, mod = abs(a), rng.randrange(0, 1 << 14), abs(b) or 1
    yield Case(f"math.modpow {base}n {exp}n {mod}n", pow(base, exp, mod))
    yield Case(f"math.isqrt {base}n", math.isqrt(base))
    n, k = rng.randrange(160), rng.randrange(180)
    yield Case(f"math.fact {n}n", math.factorial(n))
    yield Case(f"math.choose {n}n {k}n", math.comb(n, k))
    yield Case(f"math.perm {n}n {k}n", math.perm(n, k))
    i = rng.randrange(800)
    yield Case(f"math.fib {i}n", fib(i))
    estimate = rng.randrange(1, 200000)
    fs = factors(estimate)
    small_divisors = [d for d in range(1, math.isqrt(estimate) + 1) if estimate % d == 0]
    divisors = sorted(set(small_divisors + [estimate // d for d in small_divisors]))
    yield Case(f"math.prime {estimate}n", "1b" if len(fs) == 1 else "0b")
    yield Case(f"math.factor {estimate}n", seq(fs))
    yield Case(f"math.divisors {estimate}n", seq(divisors))

    c = [Fraction(rng.randrange(-10, 11), rng.randrange(1, 10)) for _ in range(rng.randrange(1, 9))]
    x = Fraction(rng.randrange(-6, 7), rng.randrange(1, 7))
    coeff = seq(q(v) for v in c)
    yield Case(f"poly.eval {coeff} {q(x)}", sum(value * x**i for i, value in enumerate(c)))
    derivative = [i * value for i, value in enumerate(c) if i]
    while derivative and not derivative[-1]:
        derivative.pop()
    yield Case(f"poly.deriv {coeff}", seq(derivative))
    integral = [0, *(value / (i + 1) for i, value in enumerate(c))]
    while integral and not integral[-1]:
        integral.pop()
    yield Case(f"poly.integ {coeff}", seq(integral))
    degree = max((i for i, value in enumerate(c) if value), default=None)
    yield Case(f"poly.degree {coeff}", "?0" if degree is None else f"?1 {degree}")


def real_cases(rng):
    x = rng.uniform(-6, 6)
    y = rng.uniform(0.1, 5.0)
    u = rng.uniform(-0.95, 0.95)
    w = rng.uniform(-1e-9, 1e-9)
    for name, value in [
        ("sin", x), ("cos", x), ("tan", x), ("atan", x), ("asin", u),
        ("acos", u), ("sinh", x), ("cosh", x), ("tanh", x),
        ("asinh", x), ("acosh", y + 1), ("atanh", u), ("exp", x),
        ("expm1", w), ("log10", y), ("log1p", w), ("gamma", y),
        ("lgamma", y), ("erf", x), ("erfc", x),
    ]:
        yield Case(f"math.{name} {r(value)}", getattr(math, name)(value), real=True)
    for name, operands, expected in [
        ("ln", [r(y)], math.log(y)),
        ("hypot", [r(x), r(y)], math.hypot(x, y)),
        ("atan2", [r(x), r(y)], math.atan2(x, y)),
        ("beta", [r(y), r(1 + abs(x))],
         math.exp(math.lgamma(y) + math.lgamma(1 + abs(x)) - math.lgamma(y + 1 + abs(x)))),
        ("pow", [r(y), r(x)], y**x),
        ("root", ["3n", r(-y)], -(y ** (1 / 3))),
        ("log", [r(2 + y), r(1 + abs(x))], math.log(1 + abs(x), 2 + y)),
        ("deg", [r(x)], math.degrees(x)), ("rad", [r(x)], math.radians(x)),
    ]:
        yield Case(f"math.{name} {' '.join(operands)}", expected, real=True)

    # The methods use a fixed analytic curve f(x) = x*x - 2.
    mid = rng.randrange(3, 14) / 8
    lo, hi = sorted((x, y))
    yield Case(f"num.diff &curve {r(mid)} 0.125r", 2 * mid, real=True, result=True)
    yield Case(f"num.integrate &curve {r(lo)} {r(hi)} opts",
               (hi**3 - lo**3) / 3 - 2 * (hi - lo), real=True, result=True)
    yield Case(f"num.interp [0r 1r 2r] [-2r -1r 0r] {r(mid)}",
               mid - 2, real=True, result=True)


HEADER = r'''<< "math";
<< "poly";
<< "num";
curve(x:r):r := - (* x x) 2r;
opts := .{ tol:=1e-12r; max:=1024n; };
!^{
'''


def run(binary, cases):
    failures = []
    with TemporaryDirectory(prefix="flux-mathesis-oracle-") as directory:
        path = Path(directory) / "oracle.flx"
        for start in range(0, len(cases), 128):
            batch = cases[start:start + 128]
            source = HEADER + "".join(f"    ! print ({case.expression});\n" for case in batch) + "};\n"
            path.write_text(source, encoding="ascii")
            process = subprocess.run([binary, "run-hir", path], text=True,
                                     capture_output=True, cwd=directory, timeout=30)
            lines = process.stdout.splitlines()
            if process.returncode or process.stderr or len(lines) != len(batch):
                raise SystemExit(f"batch {start}: {process.stderr}\n"
                                 f"expected {len(batch)} outputs, got {len(lines)}\n{source}")
            for case, got in zip(batch, lines, strict=True):
                if case.result and not got.startswith("ok "):
                    failures.append(f"{case.expression}: expected successful result, got {got}")
                    continue
                value = got.removeprefix("ok ") if case.result else got
                if case.real:
                    # Avoid a unit absolute tolerance masking zeroed-out small
                    # inputs (notably expm1/log1p and the tail of erfc).
                    atol = 2e-10 if case.result else 1e-300
                    try:
                        matches = math.isclose(float(value), case.expected,
                                               rel_tol=2e-11, abs_tol=atol)
                    except ValueError:
                        matches = False
                else:
                    matches = value == str(case.expected)
                if not matches:
                    failures.append(f"{case.expression}: expected {case.expected}, got {value}")
    if failures:
        raise SystemExit("\n".join(failures))
    return len(cases)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("flux", type=Path)
    cli.add_argument("--rounds", type=int, default=24)
    cli.add_argument("--seed", type=int, default=0xF018)
    args = cli.parse_args()
    if args.rounds < 1:
        cli.error("rounds must be at least one")
    rng = Random(args.seed)
    cases = []
    for _ in range(args.rounds):
        cases.extend(exact_cases(rng))
        cases.extend(real_cases(rng))
    print(f"{run(args.flux.resolve(strict=True), cases)} Mathesis oracle outputs matched")


if __name__ == "__main__":
    main()
