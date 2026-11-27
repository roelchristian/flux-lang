#!/usr/bin/env python3
"""Check wide Flux numerics using Python int, Fraction, Decimal and binary64."""

import argparse
from decimal import Decimal, localcontext
from fractions import Fraction
from pathlib import Path
from random import Random
import struct
import subprocess
from tempfile import TemporaryDirectory


def integer(value):
    return f"{value}z"


def rational(value):
    return f"(/ {integer(value.numerator)} {integer(value.denominator)})"


def scaled(coefficient, scale):
    sign = "-" if coefficient < 0 else ""
    digits = str(abs(coefficient))
    if not scale:
        return sign + digits
    digits = digits.zfill(scale + 1)
    return f"{sign}{digits[:-scale]}.{digits[-scale:]}"


def decimal_literal(coefficient, scale):
    return scaled(coefficient, scale) + "d"


def bits(value):
    return struct.pack(">d", value)


def binary64(value):
    return struct.unpack(">d", value.to_bytes(8, "big"))[0]


def decimal_text(value):
    # Exact decimal has no distinct negative zero; preserve the quantum.
    return format(value.copy_abs() if not value else value, "f")


def specimens():
    rng = Random(0xF105)
    for _ in range(32):
        a = rng.randrange(-(1 << 290), 1 << 291)
        b = rng.randrange(-(1 << 220), 1 << 221) or 1
        u = rational(Fraction(a, (1 << 248) + 13))
        v = rational(Fraction(b, (1 << 227) - 1))
        af = Fraction(a, (1 << 248) + 13)
        bf = Fraction(b, (1 << 227) - 1)
        for symbol, result in [("+", a + b), ("-", a - b), ("*", a * b),
                               ("//", a // b), ("%", a % b)]:
            yield f"{symbol} {integer(a)} {integer(b)}", str(result), str
        yield f"+ {abs(a)}n {abs(b)}n", str(abs(a) + abs(b)), str
        for symbol, result in [("+", af + bf), ("-", af - bf),
                               ("*", af * bf), ("/", af / bf)]:
            yield f"{symbol} {u} {v}", result, Fraction

    with localcontext() as ctx:
        ctx.prec = 1000
        for _ in range(40):
            a = rng.randrange(-(1 << 200), 1 << 200)
            b = rng.randrange(-(1 << 200), 1 << 200) or 1
            sa, sb = rng.randrange(20), rng.randrange(20)
            ad = Decimal(scaled(a, sa))
            bd = Decimal(scaled(b, sb))
            src_a = decimal_literal(a, sa)
            src_b = decimal_literal(b, sb)
            for symbol, result in [("+", ad + bd), ("-", ad - bd), ("*", ad * bd)]:
                yield f"{symbol} {src_a} {src_b}", decimal_text(result), str
            exponent = rng.randrange(1, 6)
            yield f"^ {src_a} {exponent}n", decimal_text(ad**exponent), str
            yield f"/ {src_a} {src_b}", Fraction(ad) / Fraction(bd), Fraction

    for scale in range(0, 12):
        for sign in (-1, 1):
            for coefficient in (0, 1, 2, 3, 10**55, 10**55 + 1):
                # Construct exact half-way inputs independently of Flux decimal.
                q = Fraction(sign * (2 * coefficient + 1), 2 * 10**scale)
                want = scaled(round(q * 10**scale), scale)
                yield f"math.quantize {scale}n {rational(q)}", want, str
        q = Fraction(rng.randrange(-(1 << 160), 1 << 160), (1 << 137) + 5)
        want = scaled(round(q * 10**scale), scale)
        yield f"math.quantize {scale}n {rational(q)}", want, str

    forwards = [
        Fraction(0), Fraction(1, 3),
        Fraction(1, 1 << 1074), Fraction(1, 1 << 1075),
        Fraction(-1, 1 << 1075), Fraction(3, 1 << 1075),
        Fraction(1) + Fraction(1, 1 << 53),
        Fraction(1) + Fraction(3, 1 << 53),
        Fraction(1, 1 << 1022) - Fraction(1, 1 << 1075),
        Fraction.from_float(binary64(0x7FEFFFFFFFFFFFFF)),
    ]
    for _ in range(150):
        n = rng.randrange(-(1 << 192), 1 << 192)
        d = rng.randrange(1, 1 << 192)
        exp = rng.randrange(-1150, 1000)
        forwards.append(Fraction(n << max(0, exp), d << max(0, -exp)))
    for q in forwards:
        yield f":r {rational(q)}", bits(float(q)), lambda text: bits(float(text))

    reals = [
        0.0, -0.0, 0.1, binary64(1), binary64(0x8000000000000001),
        binary64(0x000FFFFFFFFFFFFF), binary64(0x0010000000000000),
        binary64(0x7FEFFFFFFFFFFFFF),
    ]
    while len(reals) < 160:
        raw = rng.getrandbits(64)
        if (raw >> 52) & 0x7FF != 0x7FF:
            reals.append(binary64(raw))
    for r in reals:
        yield f":q {r!r}r", Fraction.from_float(r), Fraction
    for r in reals[:24]:
        yield f":d {r!r}r", decimal_text(Decimal.from_float(r)), str


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("flux", type=Path)
    args = parser.parse_args()
    binary = args.flux.resolve(strict=True)
    cases = list(specimens())
    body = "".join(f"    ^? ! io.print ({expr});\n" for expr, _, _ in cases)
    source = '<< "io"; << "math";\n!{ ^{\n' + body + '    ()\n} };\n'
    with TemporaryDirectory(prefix="flux-exact-numeric-") as work:
        root = Path(work) / "oracle.flx"
        root.write_text(source, encoding="ascii")
        proc = subprocess.run(
            [binary, "run-hir", root], text=True, capture_output=True, check=False
        )
        if proc.returncode:
            raise SystemExit(proc.stderr)
        lines = proc.stdout.splitlines()
        if len(lines) != len(cases):
            raise SystemExit(f"expected {len(cases)} outputs, got {len(lines)}")
        for (expr, want, convert), text in zip(cases, lines):
            got = convert(text)
            if want != got:
                raise SystemExit(f"{expr}: expected {want}, got {got} ({text!r})")
    print(f"{len(cases)} independent numerical specimens matched")


if __name__ == "__main__":
    main()
