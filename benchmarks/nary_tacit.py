#!/usr/bin/env python3
"""Check n-ary tacit composition against explicit functions and exact arithmetic."""
from argparse import ArgumentParser
from fractions import Fraction
from pathlib import Path
from random import Random
from subprocess import run
from tempfile import TemporaryDirectory


def literal(value):
    return f"(/ {value.numerator}q {value.denominator}q)"


def main():
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("flux", type=Path)
    parser.add_argument("--seed", type=int, default=8316)
    options = parser.parse_args()
    flux = options.flux.resolve(strict=True)
    rng = Random(options.seed)
    values = [
        tuple(Fraction(rng.randrange(-500, 500), rng.randrange(1, 45)) for _ in range(3))
        for _ in range(144)
    ]
    prelude = """affine(a,b,c:q):q := + (* a b) c;
calc/3:q->q->q->q := &{3: @* + (@* * $1 $2) $3 };
sum(a,b,c:q):q := + a + b c;
sq/1:q->q := &{ @* * . . };
vecnorm/3:q->q->q->q := &{3: @{ sum; @. sq $1; @. sq $2; @. sq $3 } };
norm(a,b,c:q):q := + (* a a) + (* b b) (* c c);
!{ ^{\n"""
    expected = []
    with TemporaryDirectory() as directory:
        entry = Path(directory, "tacit.flx")
        for start in range(0, len(values), 12):
            lines = [prelude]
            expected.clear()
            for a, b, c in values[start:start + 12]:
                args = " ".join(map(literal, (a, b, c)))
                for f in ["affine", "calc", "norm", "vecnorm"]:
                    lines.append(f"^? ! print ({f} {args});\n")
                expected.extend([a * b + c] * 2 + [a*a + b*b + c*c] * 2)
            lines.append("()\n} };\n")
            entry.write_text("".join(lines), encoding="ascii")
            for mode in [[], ["run-hir"]]:
                result = run([flux, *mode, entry], capture_output=True, text=True)
                if result.returncode:
                    raise SystemExit(result.stderr)
                actual = [Fraction(line) for line in result.stdout.splitlines()]
                if actual != expected or result.stderr:
                    raise SystemExit(f"mismatch at case {start}: {result}")
    print(f"{len(values) * 4} exact explicit/tacit results matched in both CLI modes")


if __name__ == "__main__":
    main()
