#!/usr/bin/env python3
"""Measure three ordered exact-key joins on growing, overlapping tables."""
import argparse
from pathlib import Path
from statistics import median
from subprocess import run
from tempfile import TemporaryDirectory
from time import perf_counter_ns


def fixture(rows):
    half = rows // 2
    source = f'''
<< "df";
left := #: .{{ k := .. 1z {rows}z; }};
right := #: .{{ k := .. {half + 1}z {rows + half}z; }};
!{{
    ok := # (*= &.k &.k left right);
    total := # (*? &.k &.k left right);
    miss := # (df.anti &.k &.k left right);
    ^= ! print (ok,total,miss) {{ _ : (); }};
}};
'''
    return source, f"({rows - half}, {rows}, {half})\n"


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("binary", type=Path)
    cli.add_argument("--runs", type=int, default=7)
    args = cli.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.runs < 3:
        cli.error("at least three measured runs are required")
    with TemporaryDirectory(prefix="flux-relational-") as work:
        path = Path(work) / "sample.flx"
        for rows in (1000, 6000, 12000):
            source, output = fixture(rows)
            path.write_text(source, encoding="ascii")
            timings = []
            for _ in range(args.runs + 1):
                start = perf_counter_ns()
                result = run([binary, "run-hir", path], capture_output=True, text=True)
                timings.append((perf_counter_ns() - start) / 1e6)
                if result.returncode or result.stdout != output:
                    raise RuntimeError(f"rows={rows}: {result.stdout!r}\n{result.stderr}")
            print(f"rows={rows} median_ms={median(timings[1:]):.3f}")


if __name__ == "__main__":
    main()
