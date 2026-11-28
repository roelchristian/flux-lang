#!/usr/bin/env python3
"""Compare Flux dense arrays against an independent exact coordinate model.

Run with a locked Flux release executable. Python's standard-library Fraction
and Decimal supply arithmetic expectations; no numerical dependency is needed.
"""
import argparse
from dataclasses import dataclass
from decimal import Decimal, localcontext
from fractions import Fraction
from itertools import product
from pathlib import Path
import random
import subprocess
import tempfile


def coords(shape):
    return product(*(range(length) for length in shape))


@dataclass
class Array:
    shape: tuple
    values: dict

    @classmethod
    def from_items(cls, shape, values):
        return cls(tuple(shape), dict(zip(coords(shape), values, strict=True)))

    @property
    def items(self):
        return [self.values[c] for c in coords(self.shape)]


@dataclass(frozen=True)
class Optional:
    value: object


def render(v):
    if isinstance(v, (tuple, list)):
        return "[" + " ".join(map(render, v)) + "]"
    if isinstance(v, Optional):
        return "?0" if v.value is None else f"?1 {render(v.value)}"
    if isinstance(v, Decimal):
        return format(v, "f")
    return str(v)


def dims(shape):
    return "[" + " ".join(f"{n}n" for n in shape) + "]"


def number(v):
    if isinstance(v, Decimal):
        return f"{v:f}d"
    if isinstance(v, Fraction):
        return f"(/ {v.numerator}q {v.denominator}q)"
    return f"{v}z"


def declare(name, a, numeric="z"):
    items = " ".join(map(number, a.items))
    return f"{name}_items:[{numeric}] := [{items}];\n{name} := form {dims(a.shape)} {name}_items;"


def broadcast(shape_a, shape_b):
    padded_a = (1,) * (len(shape_b) - len(shape_a)) + tuple(shape_a)
    padded_b = (1,) * (len(shape_a) - len(shape_b)) + tuple(shape_b)
    output = []
    for x, y in zip(padded_a, padded_b, strict=True):
        if x != y and x != 1 and y != 1:
            raise ValueError((shape_a, shape_b))
        output.append(y if x == 1 else x)
    return tuple(output)


def project(coord, shape):
    return tuple(0 if dim == 1 else c for c, dim in zip(coord[len(coord) - len(shape):], shape))


def subtract(a, b):
    shape = broadcast(a.shape, b.shape)
    return Array(shape, {c: a.values[project(c, a.shape)] - b.values[project(c, b.shape)]
                         for c in coords(shape)})


def permute(a, axes):
    shape = tuple(a.shape[i] for i in axes)
    return Array(shape, {tuple(c[i] for i in axes): v for c, v in a.values.items()})


def section(a, starts, counts):
    shape = tuple(max(0, min(n - s, k)) for n, s, k in zip(a.shape, starts, counts, strict=True))
    return Array(shape, {c: a.values[tuple(i+s for i, s in zip(c, starts, strict=True))]
                         for c in coords(shape)})


def axis_reductions(a, axis, seed):
    frame = a.shape[:axis] + a.shape[axis+1:]
    unseeded, seeded, scans = {}, {}, {}
    for frame_coord in coords(frame):
        def point(j):
            return frame_coord[:axis] + (j,) + frame_coord[axis:]
        line = [a.values[point(j)] for j in range(a.shape[axis])]
        unseeded[frame_coord] = Optional(line[0] - sum(line[1:]) if line else None)
        seeded[frame_coord] = seed - sum(line)
        scans.update({point(j): seed - sum(line[:j]) for j in range(len(line) + 1)})
    scan_shape = a.shape[:axis] + (a.shape[axis] + 1,) + a.shape[axis+1:]
    return Array(frame, unseeded), Array(frame, seeded), Array(scan_shape, scans)


def serial(a):
    return f"({render(a.shape)}, {render(a.items)})"


def inspect(*names):
    return [f"(dim {name}, ,. {name})" for name in names]


def suite(rng, rounds):
    """Operations emitted in source order, matched line-by-line to the model."""
    for i in range(rounds):
        # Include scalars, zero axes and singleton expansion without relying on strides.
        sa = tuple(rng.randrange(4) for _ in range(rng.randrange(5)))
        sb = tuple(rng.choice((1, n)) for n in sa)
        if i % 2:
            sb = (rng.randrange(1, 4),) + sb
        if i % 3:
            sb = sb[rng.randrange(len(sb) + 1):]
        sf = tuple(rng.randrange(4) for _ in range(rng.randrange(1, 5)))
        make = lambda shape: Array.from_items(shape, (rng.randrange(-30, 32) for _ in coords(shape)))
        a, b, f = map(make, (sa, sb, sf))
        axis = rng.randrange(len(sf))
        axes = list(range(len(sf)))
        rng.shuffle(axes)
        starts = [rng.randrange(n + 2) for n in sf]
        counts = [rng.randrange(n + 2) for n in sf]
        red, fold, scan = axis_reductions(f, axis, 17)
        defines = "\n".join((
            declare("a", a), declare("b", b), declare("f", f),
            "x := - a b;",
            f"r := arr.red {axis + 1}n &minus f;",
            f"g := arr.fold {axis + 1}n &minus 17z f;",
            f"s := arr.scana {axis + 1}n &minus 17z f;",
            f"p := arr.axes {dims([n + 1 for n in axes])} f;",
            f"v := arr.sect {dims([s + 1 for s in starts])} {dims(counts)} f;",
        ))
        outputs = [subtract(a, b), red, fold, scan, permute(f, axes), section(f, starts, counts)]
        yield f"axes/broadcast {i}", defines, inspect("x", "r", "g", "s", "p", "v"), list(map(serial, outputs))

        # Standard-library exact arithmetic with randomized rectangular products.
        m, k, n = (rng.randrange(4) for _ in range(3))
        if i % 2:
            domain = "q"
            zero = Fraction(0)
            gen = lambda: Fraction(rng.randrange(-8, 9), rng.randrange(1, 7))
        else:
            domain = "d"
            zero = Decimal(0)
            gen = lambda: Decimal(rng.randrange(-80, 81)).scaleb(-rng.randrange(0, 5))
        a = Array.from_items((m, k), (gen() for _ in range(m * k)))
        b = Array.from_items((k, n), (gen() for _ in range(k * n)))
        u = Array.from_items((k,), (gen() for _ in range(k)))
        v = Array.from_items((k,), (gen() for _ in range(k)))
        mat = Array((m, n), {(r, c): sum((a.values[r, j] * b.values[j, c] for j in range(k)), zero)
                            for r, c in coords((m, n))})
        diag = Array.from_items((min(m, k),), (a.values[j, j] for j in range(min(m, k))))
        dot = sum((x*y for x, y in zip(u.items, v.items, strict=True)), zero)
        defines = "\n".join((declare("a", a, domain), declare("b", b, domain),
                             declare("u", u, domain), declare("v", v, domain),
                             "m := arr.mul a b;", "d := arr.diag a;"))
        yield f"{domain} contractions {i}", defines, [*inspect("m", "d"), "(arr.dot u v)"], [
            serial(mat), serial(diag), render(dot)
        ]


HEADER = '<< "arr";\nminus(a,b:z):z := - a b;\n'


def check(binary, rounds, seed):
    rng = random.Random(seed)
    successes = 0
    with localcontext() as decimal_context, tempfile.TemporaryDirectory(prefix="flux-array-oracle-") as d:
        decimal_context.prec = 100
        path = Path(d, "oracle.flx")
        for name, declarations, expressions, want in suite(rng, rounds):
            lines = [f"    ^? ! print {expr};" for expr in expressions]
            program = HEADER + declarations + "\n!{ ^{\n" + "\n".join(lines) + "\n    ()\n} };\n"
            path.write_text(program, encoding="ascii")
            result = subprocess.run([binary, "run-hir", path], capture_output=True,
                                    cwd=d, text=True, timeout=15)
            actual = result.stdout.splitlines()
            if result.returncode != 0 or result.stderr or actual != want:
                raise SystemExit(f"{name}: {result.stderr}expected {want!r}, got {actual!r}\n{program}")
            successes += len(want)
    return successes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--rounds", type=int, default=40)
    parser.add_argument("--seed", type=int, default=1514)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("rounds must be positive")
    count = check(args.binary.resolve(strict=True), args.rounds, args.seed)
    print(f"{count} exact array/shape oracle outputs matched across {2 * args.rounds} scripts")


if __name__ == "__main__":
    main()
