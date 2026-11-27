#!/usr/bin/env python3
"""Run shared specimens against a Draft 0.3 oracle and the Draft 0.4 binary."""
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ("permutation.flx", "observations.flx", "csv-grid.flx", "pipeline.flx", "monitoring.flx")
SPECIMENS = {
    "exact": '<< "io"; !{ ! io.print + [1 2 3] 4q; ! io.print / 3 7; };',
    "order": '<< "io"; f(x:z):z := + x 1; !{ ! io.print |{ 4; f $; + $ $ }; };',
    "exit": "!{ 17 };",
    "shape-failure": '<< "io"; !{ ! io.print * [1 2 3] [4 5]; };',
    "short-circuit": '<< "io"; !{ ! io.print ?{ 1b : 2q; _ : / 1 0; }; };',
}


def invoke(executable, name, source, *, typed=False):
    with TemporaryDirectory(prefix="flux-diff-") as name_of_directory:
        directory = Path(name_of_directory)
        shutil.copytree(ROOT / "examples", directory / "examples")
        if source is None:
            script = f"examples/{name}"
        else:
            script = name + ".flx"
            (directory / script).write_text(source, encoding="ascii")
        checked = subprocess.run(
            [executable, "check", script], cwd=directory, capture_output=True, check=False
        )
        process = subprocess.run(
            [executable, *(["run-hir"] if typed else []), script],
            cwd=directory, capture_output=True, check=False
        )
        response = directory / "summary.csv"
        return (
            checked.returncode,
            checked.stdout,
            process.returncode,
            process.stdout.replace(b"\r\n", b"\n"),
            response.read_bytes() if response.exists() else None,
        )


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: differential.py DRAFT03_BINARY DRAFT04_BINARY")
    oracle, candidate = (Path(arg).resolve(strict=True) for arg in sys.argv[1:])
    for name, source in [*((n, None) for n in EXAMPLES), *SPECIMENS.items()]:
        expected = invoke(oracle, name, source)
        actual = invoke(candidate, name, source, typed=True)
        if expected != actual:
            print(f"{name}: oracle={expected!r}, candidate={actual!r}", file=sys.stderr)
            return 1
        print(f"{name}: match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
