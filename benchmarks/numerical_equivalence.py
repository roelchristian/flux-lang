#!/usr/bin/env python3
"""Compare two Flux release binaries with optimization both on and off.

The independent, deliberately small Python oracle checks complete outputs, not
only checksums. Directed specimens also pin failure category, originating span
and effects before failure; differences in diagnostics are not discarded.
"""

import argparse
from dataclasses import asdict, dataclass
from fractions import Fraction
from itertools import product
import json
from math import prod
import os
from pathlib import Path
from random import Random
import subprocess
import sys
from tempfile import TemporaryDirectory


SEED = 0xF1021
TIMEOUT_SECONDS = 10


@dataclass(frozen=True)
class Case:
    name: str
    source: str
    stdout: str = ""
    status: int = 0
    diagnostic: tuple = ()
    mode: str = "run-hir"
    limits: tuple = ()


@dataclass(frozen=True)
class Result:
    status: int
    stdout: str
    stderr: str


def vector(items, suffix=""):
    return "[" + " ".join(f"{item}{suffix}" for item in items) + "]"


def dims(*shape):
    return vector(shape, "n")


def tupled(*items):
    return "(" + ", ".join(map(str, items)) + ")"


def expr_program(declarations, outputs):
    return declarations + "\n!^{\n" + "".join(f"    ! print {out};\n" for out in outputs) + "};\n"


def location(source, anchor):
    index = source.index(anchor)
    line = source.count("\n", 0, index) + 1
    column = index - source.rfind("\n", 0, index)
    return f"case.flx:{line}:{column}:"


def fixed():
    source = expr_program(
        '<< "arr";\nempty:[z] := [];\n'
        'cells := form [2n 0n 3n] empty;\n'
        'sample := arr.sect [1n 1n 1n] [9n 9n 9n] (arr.axes [1n 3n 2n] cells);\n'
        'scalar := form [] [12z];',
        ["(dim sample, # sample, ,. sample)",
         "(dim (+/ sample), ,. (+/ sample), ,. (@/ &- sample))",
         "(dim scalar, # scalar, .: [] scalar, ,. (form [1n] scalar))"],
    )
    yield Case("zero-volume-and-rank-zero", source, concat_lines(
        "([2 3 0], 0, [])",
        "([2 3], [0 0 0 0 0 0], [?0 ?0 ?0 ?0 ?0 ?0])",
        "([], 1, ?1 12, [12])",
    ))

    source = expr_program(
        '<< "arr";\n'
        'empty:[z] := [];\n'
        'left := form [2n 0n] empty;\n'
        'right := form [0n 3n] empty;\n'
        'd := form [0n] empty;',
        ["(dim (arr.mul left right), ,. (arr.mul left right), arr.dot d d)"],
    )
    yield Case("empty-exact-contraction", source, "([2 3], [0 0 0 0 0 0], 0)\n")

    source = expr_program(
        '<< "arr";\n'
        'a := form [3n 2n] [-0.0r -0.0r 0.0r -0.0r -0.0r 0.0r];\n'
        'b := arr.sect [1n 1n] [2n 3n] (|: a);',
        ["(dim b, ,. b, ,. (-: (-: (-: b))))",
         "(,. (arr.red 2n &+ b), ,. (arr.scana 2n &+ 0r b))"],
    )
    yield Case("real-signed-zero-across-views", source, concat_lines(
        "([2 3], [-0 0 -0 -0 -0 0], [0 -0 0 0 0 -0])",
        "([?1 0 ?1 0], [0 0 0 0 0 0 0 0])",
    ))

    source = expr_program(
        '<< "arr";\n'
        'decimal := form [2n 1n] [1.20d -0.10d];\n'
        'z := form [1n 2n] [2z -3z];\n'
        'q := form [1n 2n] [/ 1q 5q / 1q 7q];',
        ["(dim (+ decimal z), ,. (+ decimal z), ,. (* decimal z))",
         "(,. (+ decimal q), ,. (/ decimal 2z))"],
    )
    yield Case("decimal-scale-and-mixed-exact-promotion", source, concat_lines(
        "([2 2], [3.20 -1.80 1.90 -3.10], [2.40 -3.60 -0.20 0.30])",
        "([7/5 47/35 1/10 3/70], [3/5 -1/20])",
    ))

    source = (
        '<< "arr";\n'
        'none:?z := ?0;\n'
        'bad(x,y:z):z := ?{ = y 0z : ?! none; _ : + x y; };\n'
        'data := form [2n 2n] [1z 2z 0z 4z];\n'
        '!^{\n'
        '    ! print "before callback";\n'
        '    ! print ,. (arr.fold 2n &bad 0z data);\n'
        '    ! print "unreachable";\n'
        '};\n'
    )
    yield Case("callback-failure-provenance", source, "before callback\n", 1,
               ("R0001", location(source, "?! none"), "cannot unwrap ?0"))

    source = (
        '<< "arr";\n'
        'm := form [2n 3n] [1z 2z 3z 4z 5z 6z];\n'
        '!^{\n'
        '    ! print "before section";\n'
        '    ! print ,. (arr.sect [1n 0n] [2n 3n] m);\n'
        '    ! print "unreachable";\n'
        '};\n'
    )
    yield Case("invalid-section-provenance", source, "before section\n", 1,
               ("R0001", location(source, "(arr.sect [1n"), "one-based"))

    source = (
        '!^{\n'
        '    a := form [2n 1n] [1.0e308r 0r];\n'
        '    b := form [3n 2n] [1r 2r 3r 4r 5r 6r];\n'
        '    ! print "before nested operation";\n'
        '    ! print ,. (+ (* a 2r) b);\n'
        '    ! print "unreachable";\n'
        '};\n'
    )
    yield Case("operand-failure-precedes-outer-shape", source, "before nested operation\n", 1,
               ("R0001", location(source, "(* a 2r"), "nonfinite"))

    source = expr_program(
        'a := form [2n 2n] [1z 2z 3z 4z];\n'
        'b := form [3n 2n] [1z 2z 3z 4z 5z 6z];',
        ['"before incompatible shape"', ",. (+ a b)", '"unreachable"'],
    )
    yield Case("incompatible-broadcast-shape", source, "before incompatible shape\n", 1,
               ("R0001", location(source, "(+ a b"), "incompatible trailing axes"))

    source = expr_program('a := form [2n] [20z 31z];',
                          ['"before exact cap"', ",. (* a 8z)", '"unreachable"'])
    yield Case("host-exact-bit-limit", source, "before exact cap\n", 1,
               ("R0001", location(source, "(* a 8z"), "resource bit limit"),
               limits=(("FLUX_MAX_EXACT_BITS", "7"),))

    source = expr_program('a := form [1n] [1z];', [",. (+ a [2z])"])
    yield Case("static-array-sequence-rejection", source, status=1,
               diagnostic=("E0200", location(source, "(+ a [2z]"),
                           "cannot mix a dense array and a sequence"),
               mode="check")


def concat_lines(*rows):
    return "".join(row + "\n" for row in rows)


def indices(shape):
    return product(*(range(width) for width in shape))


def generated_views(rng, index):
    # The oracle addresses logical coordinates directly, rather than sharing
    # runtime stride, offset, odometer, or broadcasting-plan arithmetic.
    grid = tuple(rng.randint(1, 4) for _ in range(3))
    cells = [rng.randint(-100, 100) for _ in range(prod(grid))]
    if index % 3 == 0:
        cells[-1] = (1 << (140 + index)) - 1
    elif index % 3 == 1:
        cells[-1] = (1 << 63) - 1
    source_cells = dict(zip(indices(grid), cells))
    perm = list(range(len(grid)))
    rng.shuffle(perm)
    shape = tuple(grid[p] for p in perm)
    starts = tuple(rng.randint(1, width + 1) for width in shape)
    counts = tuple(rng.randint(0, width + 2) for width in shape)
    clipped = tuple(min(width - start + 1, n) for width, start, n in zip(shape, starts, counts))
    offsets = tuple(start - 1 for start in starts)
    broadcast = [rng.randint(-13, 13) for _ in range(clipped[-1])]
    # Exercise both arbitrary-width operands and compact overflow fallback.
    if index % 3 != 2:
        starts = (1, 1, 1)
        counts = clipped = shape
        offsets = (0, 0, 0)
        broadcast = [rng.randint(-13, 13) for _ in range(clipped[-1])]
        if index % 3 == 1:
            broadcast[-1] = 2
    adjusted = {}
    lowered = []
    for position in indices(clipped):
        address = [0] * len(grid)
        for p, dim in enumerate(perm):
            address[dim] = position[p] + offsets[p]
        value = source_cells[tuple(address)]
        adjusted[position] = value + broadcast[position[-1]]
        lowered.append(Fraction(value, 3))
    spans = []
    for lane in indices(clipped[:-1]):
        row = [adjusted[(*lane, i)] for i in range(clipped[-1])]
        spans.append("?0" if not row else f"?1 {row[0] - sum(row[1:])}")
    declaration = (
        '<< "arr";\n'
        f'a := form {dims(*grid)} {vector(cells, "z")};\n'
        f'p := arr.axes {dims(*(axis + 1 for axis in perm))} a;\n'
        f's := arr.sect {dims(*starts)} {dims(*counts)} p;\n'
        f'xs:[z] := {vector(broadcast, "z")};\n'
        f'delta := form {dims(clipped[-1])} xs;\n'
        'x := + s delta;\n'
        f'flat := form {dims(prod(clipped))} s;\n'
    )
    source = expr_program(declaration, [
        "(dim x, ,. x)", "(dim flat, ,. (/ flat 3z))", "(dim (@/ &- x), ,. (@/ &- x))",
    ])
    return Case(f"generated-views-{index:02d}", source, concat_lines(
        tupled(vector(clipped), vector(adjusted.values())),
        tupled(vector([prod(clipped)]), vector(lowered)),
        tupled(vector(clipped[:-1]), vector(spans)),
    ))


def real_literal(value):
    return repr(float(value)) + "r"


def left_dot(a, b):
    # Python's built-in sum() can use compensated summation on newer Python.
    # Keep separate binary64 multiply and add and never use FMA/reassociation.
    total = 0.0
    for x, y in zip(a, b):
        total = total + (float(x) * float(y))
    return Fraction.from_float(total)


def generated_contractions(rng, index):
    m, k, n = rng.randint(2, 4), rng.randint(3, 6), rng.randint(2, 4)
    a = [[rng.randint(-8, 8) for _ in range(k)] for _ in range(m)]
    b = [[rng.randint(-8, 8) for _ in range(n)] for _ in range(k)]
    # One row/column distinguish left-fold from reassociated accumulation;
    # another tests a product rounding barrier (no fused multiply-add).
    a[0][:3], a[1][:3] = [2.0**53, 1.0, -(2.0**53)], [-1.0, 1.0 + 2.0**-27, 0.0]
    for i, row in enumerate(b):
        row[0] = 1.0 if i < 3 else 0.0
        row[1] = ([1.0, 1.0 - 2.0**-27, 0.0][i] if i < 3 else 0.0)
    transposed = (real_literal(a[i][j]) for j in range(k) for i in range(m))
    padded = [real_literal(999)] * (n + 2)
    for row in b:
        padded.extend([real_literal(-999), *map(real_literal, row), real_literal(999)])
    declaration = (
        '<< "arr";\n'
        f'a := |: (form {dims(k, m)} {vector(transposed)});\n'
        f'pad := form {dims(k + 1, n + 2)} {vector(padded)};\n'
        f'b := arr.sect [2n 2n] {dims(k, n)} pad;\n'
        'c := arr.mul a b;\n'
    )
    # The exact rational render exposes every binary64 bit except sign of zero,
    # which is checked independently by the fixed signed-zero specimen.
    source = expr_program(declaration, ["(dim c, ,. (:q c), ,. (:q (arr.diag c)))"])
    columns = list(zip(*b))
    values = [[left_dot(row, col) for col in columns] for row in a]
    return Case(f"generated-real-contractions-{index:02d}", source, concat_lines(
        tupled(vector([m, n]), vector(v for row in values for v in row),
               vector(values[i][i] for i in range(min(m, n)))),
    ))


def cases(count):
    yield from fixed()
    rng = Random(SEED)
    for index in range(count):
        yield generated_views(rng, index)
        yield generated_contractions(rng, index)


def invoke(binary, case, mode, optimized):
    # No caller configuration may influence the baseline or hide a fault.
    env = {key: value for key, value in os.environ.items() if not key.startswith("FLUX_")}
    env.update(case.limits)
    env["FLUX_OPTIMIZE"] = str(optimized)
    with TemporaryDirectory(prefix="flux-equiv-") as directory:
        root = Path(directory).resolve()
        source = root / "case.flx"
        source.write_bytes(case.source.encode("ascii"))
        args = [binary, *([mode] if mode else []), source.name]
        process = subprocess.run(args, cwd=root, env=env, input=b"", capture_output=True,
                                 check=False, timeout=TIMEOUT_SECONDS)
        diagnostic = process.stderr.decode("utf-8").replace(str(source), source.name)
        return Result(process.returncode, process.stdout.decode("utf-8"), diagnostic)


def check_expected(label, case, result):
    errors = []
    if result.status != case.status:
        errors.append(f"{label}: exit status {result.status}; expected {case.status}")
    # Rust normally writes LF to pipes on all platforms. Permit a Windows
    # newline translation here only; the inter-arm result still compares raw.
    stdout = result.stdout.replace("\r\n", "\n") if os.name == "nt" else result.stdout
    if stdout != case.stdout:
        errors.append(f"{label}: stdout does not equal the independent expectation")
    if not case.diagnostic and result.stderr:
        errors.append(f"{label}: unexpected stderr")
    first_diagnostic = result.stderr.partition("\n")[0]
    for fragment in case.diagnostic:
        if fragment not in first_diagnostic:
            errors.append(f"{label}: missing first diagnostic fragment {fragment!r}")
    return errors


def run_case(arms, case):
    results, errors = {}, []
    modes = (case.mode, "") if case.mode == "run-hir" else (case.mode,)
    reference = None
    reference_label = None
    for mode in modes:
        for name, binary in arms:
            for optimize in (0, 1):
                label = f"{name}/{mode or 'ordinary'}/opt={optimize}"
                try:
                    result = invoke(binary, case, mode, optimize)
                except (OSError, UnicodeError, subprocess.TimeoutExpired) as error:
                    errors.append(f"{label}: could not obtain a valid result: {error}")
                    continue
                results[label] = asdict(result)
                errors.extend(check_expected(label, case, result))
                if reference is None:
                    reference, reference_label = result, label
                elif result != reference:
                    differences = [field for field in ("status", "stdout", "stderr")
                                   if getattr(reference, field) != getattr(result, field)]
                    errors.append(f"{label} differs from {reference_label}: {', '.join(differences)}")
    return {
        "case": case.name,
        "limits": dict(case.limits),
        "expected": {"status": case.status, "stdout": case.stdout, "diagnostic": case.diagnostic},
        "runs": results,
        "errors": errors,
        **({"source": case.source} if errors else {}),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path, help="locked release binary from the baseline checkout")
    parser.add_argument("candidate", type=Path, help="locked release binary from the candidate checkout")
    parser.add_argument("--cases", type=int, default=12, metavar="N",
                        help="deterministic generated sets in addition to fixed cases (0: smoke only)")
    parser.add_argument("--report", type=Path, help="write a JSON record of all observations")
    args = parser.parse_args()
    if not 0 <= args.cases <= 64:
        parser.error("--cases must be between 0 and 64")
    try:
        arms = [(name, getattr(args, name).resolve(strict=True)) for name in ("baseline", "candidate")]
    except OSError as error:
        parser.error(str(error))
    records = []
    for case in cases(args.cases):
        record = run_case(arms, case)
        records.append(record)
        if record["errors"]:
            print(f"{case.name}: FAIL\n{case.source}", file=sys.stderr)
            print(json.dumps(record, indent=2), file=sys.stderr)
            # Stop on the first mismatching specimen, particularly for timeouts.
            break
    if args.report:
        report = {
            "binaries": dict((name, str(binary)) for name, binary in arms),
            "seed": SEED,
            "generated_sets": args.cases,
            "cases": records,
        }
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    passed = not records[-1]["errors"]
    if passed:
        print(f"{len(records)} numerical equivalence specimens matched "
              f"({sum(len(r['runs']) for r in records)} process outcomes)")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
