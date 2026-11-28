#!/usr/bin/env python3
"""Check Flux linear algebra using independent exact arithmetic and residuals."""
import argparse
from fractions import Fraction
from itertools import permutations
from math import isfinite, prod
import os
from pathlib import Path
from random import Random
import subprocess
from tempfile import TemporaryDirectory


def matrix(rows, domain="z"):
    r, c = len(rows), len(rows[0]) if rows else 0
    data = " ".join(f"{x}{domain}" for row in rows for x in row)
    return f"form [{r}n {c}n] [{data}]"


def sign(p):
    return (-1) ** sum(p[i] > p[j] for i in range(len(p)) for j in range(i + 1, len(p)))


def determinant(rows):
    # Direct Leibniz sum, deliberately independent of pivot/elimination code.
    n = len(rows)
    return sum(sign(p) * prod(rows[i][p[i]] for i in range(n)) for p in permutations(range(n)))


def multiply(a, b):
    return [[sum(x * y for x, y in zip(row, col)) for col in zip(*b)] for row in a]


def transpose(a):
    return list(map(list, zip(*a)))


def identity(n):
    return [[float(i == j) for j in range(n)] for i in range(n)]


def same_shape_close(actual, expected, label):
    if len(actual) != len(expected) or any(len(a) != len(e) for a, e in zip(actual, expected)):
        raise ValueError(f"{label}: reconstruction shape differs")
    scale = max(1.0, *(abs(value) for row in expected for value in row))
    defect = max(abs(a - e) for A, E in zip(actual, expected) for a, e in zip(A, E))
    if not isfinite(defect) or defect > 3e-12 * scale:
        raise ValueError(f"{label}: residual {defect!r} (scale {scale!r})")


def numbers(line, cast=float):
    if not (line.startswith("[") and line.endswith("]")):
        raise ValueError(f"expected a rendered sequence, found {line!r}")
    return list(map(cast, line[1:-1].split()))


def as_matrix(line, rows, columns, cast=float):
    cells = numbers(line, cast)
    if len(cells) != rows * columns:
        raise ValueError(f"expected {rows} by {columns} cells, got {len(cells)}")
    return [cells[i * columns:(i + 1) * columns] for i in range(rows)]


def evaluate(binary, folder, definitions, results, optimize):
    source = folder / "verify.flx"
    actions = "".join(f"    ! print {expr};\n" for expr in results)
    source.write_text('<< "lin";\n' + definitions + '\n!^{\n' + actions + '};\n', encoding="ascii")
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("FLUX_")}
    env["FLUX_OPTIMIZE"] = str(optimize)
    ran = subprocess.run([binary, "run-hir", source], env=env, capture_output=True, text=True,
                         check=False, timeout=10)
    if ran.returncode != 0 or ran.stderr:
        raise RuntimeError(f"run-hir failed: {ran.stderr}\n{source.read_text()}")
    return ran.stdout.splitlines()


def verify_exact(binary, folder, rng, rounds):
    done = 0
    while done < rounds:
        n = rng.randint(1, 5)
        a = [[rng.randint(-12, 12) for _ in range(n)] for _ in range(n)]
        det = determinant(a)
        if not det:
            continue
        nums = [rng.randint(-20, 20) for _ in range(n)]
        den = rng.randint(2, 17)
        rhs = [sum(x * y for x, y in zip(row, nums)) for row in a]
        definitions = (
            f"a := {matrix(a)};\n"
            + "b:array[q] := form [" + str(n) + "n] ["
            + " ".join(f"/ {x}q {den}q" for x in rhs) + "];\n"
            + "d := lin.det a; r := lin.rank a;\nmissing:?array[q] := ?0;\n"
            + "s := ^= lin.solve (:q a) b { _ : ?! missing; };"
        )
        expected = [Fraction(x, den) for x in nums]
        for opt in (0, 1):
            actual = evaluate(binary, folder, definitions, ["d", "r", ",. s"], opt)
            if actual[:2] != [f"ok {det}", f"ok {n}"] or numbers(actual[2], Fraction) != expected:
                raise ValueError(f"exact case {done} opt={opt}: {actual!r}; expected {det}, {n}, {expected!r}")
        done += 1


def verify_real(binary, folder):
    source = [[12, -3, 6], [6, 10, 9], [3, 0, 8]]
    a = [[float(x) for x in row] for row in source]
    spd = multiply(a, transpose(a))
    definitions = (
        f"a := {matrix(source, 'r')};\n"
        + f"s := {matrix([[int(x) for x in row] for row in spd], 'r')};\n"
        + 'blank:[r] := []; m := form [0n 0n] blank;\nmissing:?array[r] := ?0;\n'
        + 'lu := ^= lin.lu a { _ : .{ p := [1n 2n 3n]; l := m; u := m; }; };\n'
        + 'qr := ^= lin.qr a { _ : .{ q := m; r := m; }; };\n'
        + 'chol := ^= lin.cholesky s { _ : m; };\n'
        + 'b := form [3n] [30r 26r 19r];\n'
        + 'fit := ^= lin.lstsq 1e-12r a b { _ : ?! missing; };'
    )
    for opt in (0, 1):
        p, l, u, q, r, chol, fit = evaluate(binary, folder, definitions,
            [".p lu", ",. .l lu", ",. .u lu", ",. .q qr", ",. .r qr", ",. chol", ",. fit"], opt)
        permutation = numbers(p, int)
        if sorted(permutation) != [1, 2, 3]:
            raise ValueError(f"invalid one-based pivot order: {p}")
        l, u, q, r, chol = (as_matrix(x, 3, 3) for x in [l, u, q, r, chol])
        same_shape_close(multiply(l, u), [a[i - 1] for i in permutation], "LU")
        same_shape_close(multiply(q, r), a, "QR")
        same_shape_close(multiply(transpose(q), q), identity(3), "orthogonality")
        same_shape_close(multiply(chol, transpose(chol)), spd, "Cholesky")
        same_shape_close(multiply(a, [[x] for x in numbers(fit)]), [[30.0], [26.0], [19.0]], "least squares")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--rounds", type=int, default=24)
    parser.add_argument("--seed", type=int, default=0xA16EB)
    args = parser.parse_args()
    if not 1 <= args.rounds <= 256:
        parser.error("--rounds must be 1–256")
    binary = args.binary.resolve(strict=True)
    with TemporaryDirectory(prefix="flux-algebra-") as temp:
        directory = Path(temp)
        verify_exact(binary, directory, Random(args.seed), args.rounds)
        verify_real(binary, directory)
    print(f"{args.rounds} independent exact systems and 5 real residual checks matched in both modes")


if __name__ == "__main__":
    main()
