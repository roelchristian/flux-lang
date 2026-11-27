#!/usr/bin/env python3
"""Time the Draft 0.4 phases separately in a Rust release build."""
import argparse
from pathlib import Path
from statistics import median
import subprocess
from tempfile import TemporaryDirectory


def workloads():
    for n in [100, 500, 1000]:
        yield f"decl-{n}", "".join(f"x{i} := {i};\n" for i in range(n))
        yield f"generic-{n}", "id(x) := x;\n" + "".join(
            f"x{i} := id {i if i % 2 == 0 else chr(34) + str(i) + chr(34)};\n" for i in range(n)
        )
    for n in [10, 50, 150]:
        yield f"depth-{n}", "x := " + "+ 1 " * n + "0;\n"
    for n in [10, 50, 150]:
        yield f"width-{n}", "r := .{ " + "".join(f"f{i} := {i}; " for i in range(n)) + "};\n"
    yield "evaluation-range", '<< "io"; !{ ! io.print +/ (.. 1 10000); };\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--runs", type=int, default=5)
    cli = parser.parse_args()
    if cli.runs < 3:
        parser.error("--runs must be at least 3")
    path = cli.binary.resolve(strict=True)
    with TemporaryDirectory(prefix="flux04-phases-") as directory:
        for name, source in workloads():
            script = Path(directory) / (name + ".flx")
            script.write_text(source, encoding="ascii")
            runs = []
            for _ in range(cli.runs + 1):
                result = subprocess.run(
                    [path, "phases", script], capture_output=True, text=True, check=False
                )
                if result.returncode:
                    print(f"{name}: {result.stderr.strip()}")
                    break
                runs.append({k: int(v) for k, v in (line.split("\t") for line in result.stderr.splitlines())})
            if len(runs) == cli.runs + 1:
                print(name, *(f"{k}={median(x[k] for x in runs[1:]):.0f}" for k in runs[0]))


if __name__ == "__main__":
    main()
