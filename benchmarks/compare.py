#!/usr/bin/env python3
"""Compare two release Flux binaries on deterministic, end-to-end workloads."""
import argparse
import csv
import json
from pathlib import Path
from statistics import median
import subprocess
from tempfile import TemporaryDirectory
from time import perf_counter_ns

from migration import fallible_io


def prepare(directory, *, modern):
    files = {}
    source = directory / "arithmetic.flx"
    source.write_text('<< "io";\n!{ ! io.print |{ .. 1 100000; ^ $ 2; +/ $ }; };\n')
    files["numeric pipeline (100,000 values)"] = ([source.name], "333338333350000\n")
    source = directory / "large-source.flx"
    source.write_text("".join(f"x{i} := + {i} (* {i} 2);\n" for i in range(4000)))
    files["lex/parse/check (4,000 declarations)"] = (["check", source.name], "")
    for groups in (32, 6000):
        data = directory / f"obs-{groups}.csv"
        with data.open("w", newline="") as handle:
            out = csv.writer(handle)
            out.writerow(["value", "bucket"])  # Deliberately not schema order.
            out.writerows((i, f"k{i % groups}") for i in range(6000))
        source = directory / f"group-{groups}.flx"
        source.write_text(f'''<< "io"; << "csv"; << "df";
obs :: {{ bucket:s; value:z; }};
summ(k:s,t:#obs) := .{{ bucket := k; total := +/ .value t; }};
!{{
  t:#obs := ! csv.read "{data.name}";
  a := df.agg &.bucket &summ t;
  ! io.print (# a);
  ! io.print (+/ .total a);
}};
''')
        files[f"typed CSV + aggregate (6,000 rows, {groups:,d} keys)"] = (
            [source.name], f"{groups}\n17997000\n")
    if modern:
        for (args, _expected) in files.values():
            path = directory / args[-1]
            path.write_text(fallible_io(path.read_text()))
    return files


def invoke(binary, args, expected, cwd):
    start = perf_counter_ns()
    process = subprocess.run([binary, *args], cwd=cwd, check=True, capture_output=True, text=True)
    elapsed = (perf_counter_ns() - start) / 1e6
    if process.stdout != expected:
        raise ValueError(f"{binary} {args}: expected {expected!r}, received {process.stdout!r}")
    return elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path, nargs="?")
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--legacy-before", action="store_true",
                        help="use pre-0.4.1 I/O source for the first binary")
    options = parser.parse_args()
    if options.runs < 3:
        parser.error("--runs must be at least 3")
    bins = [p.resolve(strict=True) for p in (options.before, options.after) if p]
    with TemporaryDirectory(prefix="flux-bench-") as temp:
        directories = [Path(temp) / str(i) for i in range(len(bins))]
        samples_by_binary = []
        for i, cwd in enumerate(directories):
            cwd.mkdir()
            samples_by_binary.append(prepare(cwd, modern=not (options.legacy_before and i == 0)))
        results = {}
        for name, (args, expected) in samples_by_binary[0].items():
            for binary, cwd in zip(bins, directories):
                invoke(binary, args, expected, cwd)
            samples = [[] for _ in bins]
            for i in range(options.runs):
                order = list(range(len(bins)))
                if i % 2:
                    order.reverse()
                for j in order:
                    samples[j].append(invoke(bins[j], args, expected, directories[j]))
            results[name] = [
                {"median_ms": round(median(sample), 3), "samples_ms": [round(x, 3) for x in sample]}
                for sample in samples
            ]
        print(json.dumps({"runs": options.runs, "results": results}, indent=2))


if __name__ == "__main__":
    main()
