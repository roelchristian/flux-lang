#!/usr/bin/env python3
"""Time strict typed frontend phases in a Rust release build.

parse includes lexing; eval includes stdout writes for scripts with an entry.
Declaration-only workloads do not execute initializers. Use compare.py for
compatible end-to-end timings including process startup and migration fallback.
"""
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
    yield "evaluation-range", '<< "io"; !{ ^{ ^? ! io.print +/ (.. 1 10000); () } };\n'
    yield "caret-recovery-10000", """
err :: { | odd; };
classify(x:z):z^err := ^{
    ?{ = (% x 2) 1 : ^: odd; _ : x; }
};
retain(x:z):z := ^= classify x { odd : 0; };
!{
    total := +/ (@: &retain (.. 1 10000));
    ?{ = total 25005000 : 0n; _ : 1n; }
};
"""



def arithmetic_workloads():
    for n in [500, 1000, 2000, 4000, 8000]:
        # Mirror the arithmetic declarations in compare.py (4000-decl case).
        yield f"arithmetic-decl-{n}", "".join(
            f"x{i} := + {i} (* {i} 2);\n" for i in range(n)
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument(
        "--arithmetic-only", action="store_true",
        help="profile the arithmetic-declaration scaling workloads from compare.py",
    )
    cli = parser.parse_args()
    if cli.runs < 3:
        parser.error("--runs must be at least 3")
    path = cli.binary.resolve(strict=True)
    with TemporaryDirectory(prefix="flux04-phases-") as directory:
        for name, source in (arithmetic_workloads() if cli.arithmetic_only else workloads()):
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
