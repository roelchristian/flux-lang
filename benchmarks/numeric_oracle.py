#!/usr/bin/env python3
"""Cross-check bounded exact/real conversions using an independent Fraction oracle."""
import argparse
from fractions import Fraction
from pathlib import Path
from random import Random
import subprocess
from tempfile import TemporaryDirectory


def specimens():
    maximum = (1 << 127) - 1
    rng = Random(0xF104)
    forwards = [(1, 3), (1, maximum), (-maximum, 1), (0, maximum)]
    for _ in range(150):
        n = rng.randrange(-(1 << 127), 1 << 127)
        d = rng.randrange(1, 1 << 127)
        forwards.append((n, d))
        forwards.append((n >> 74, (d >> 74) + 1))
    reals = [0.0, 0.1, 0.5, -0.5, 0.8574633481823224]
    reals.extend(rng.uniform(-(2.0**72), 2.0**72) for _ in range(80))
    reals.extend(rng.uniform(-1.0, 1.0) for _ in range(80))
    ops = [(":r (/ {} {})".format(n, d), float(Fraction(n, d)), float)
           for n, d in forwards]
    return ops + [(f":q {x!r}r", Fraction(x), Fraction) for x in reals]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("flux", type=Path)
    args = parser.parse_args()
    binary = args.flux.resolve(strict=True)
    ops = specimens()
    with TemporaryDirectory(prefix="flux-numeric-") as directory:
        path = Path(directory) / "oracle.flx"
        source = '<< "io";\n!{\n' + ''.join(
            f"    ! io.print ({op});\n" for op, _, _ in ops
        ) + '};\n'
        path.write_text(source, encoding="ascii")
        proc = subprocess.run([binary, "run-hir", path], text=True, capture_output=True, check=False)
        if proc.returncode:
            raise SystemExit(proc.stderr)
        lines = proc.stdout.splitlines()
        if len(lines) != len(ops):
            raise SystemExit(f"expected {len(ops)} outputs, got {len(lines)}")
        for (op, expected, convert), text in zip(ops, lines):
            got = convert(text)
            if got != expected:
                raise SystemExit(f"{op}: expected {expected}, got {got}")
    print(f"{len(ops)} numeric conversion specimens matched")


if __name__ == "__main__":
    main()
