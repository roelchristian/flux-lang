#!/usr/bin/env python3
"""Run shared specimens against a Draft 0.3/0.4 oracle and the Draft 0.4.1 binary."""
import argparse
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory

from migration import fallible_io

ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ("permutation.flx", "observations.flx", "csv-grid.flx", "pipeline.flx", "monitoring.flx")
SPECIMENS = {
    "exact": '<< "io"; !{ ! io.print + [1 2 3] 4q; ! io.print / 3 7; };',
    "order": '<< "io"; f(x:z):z := + x 1; !{ ! io.print |{ 4; f $; + $ $ }; };',
    "exit": "!{ 17 };",
    "shape-failure": '<< "io"; !{ ! io.print * [1 2 3] [4 5]; };',
    "short-circuit": '<< "io"; !{ ! io.print ?{ 1b : 2q; _ : / 1 0; }; };',
}


def invoke(executable, name, source, *, typed=False, legacy=False, oracle_tree=None):
    with TemporaryDirectory(prefix="flux-diff-") as name_of_directory:
        directory = Path(name_of_directory)
        shutil.copytree(ROOT / "tests" / "fixtures" / "pre-corpus", directory / "examples")
        if source is None:
            script = f"examples/{name}"
            if legacy:
                if oracle_tree is not None:
                    (directory / script).write_bytes((oracle_tree / script).read_bytes())
                else:
                    previous = subprocess.check_output(
                        ["git", "show", f"e3136d7942a40659478907aefd1791dd3d6db04d:{script}"],
                        cwd=ROOT,
                    )
                    (directory / script).write_bytes(previous)
        else:
            script = name + ".flx"
            (directory / script).write_text(fallible_io(source) if not legacy else source, encoding="ascii")
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
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("oracle", type=Path)
    cli.add_argument("candidate", type=Path)
    cli.add_argument("--oracle-tree", type=Path, help="checkout with pre-0.4.1 examples")
    args = cli.parse_args()
    oracle, candidate = (p.resolve(strict=True) for p in (args.oracle, args.candidate))
    for name, source in [*((n, None) for n in EXAMPLES), *SPECIMENS.items()]:
        expected = invoke(oracle, name, source, legacy=True, oracle_tree=args.oracle_tree)
        actual = invoke(candidate, name, source, typed=True)
        if expected != actual:
            print(f"{name}: oracle={expected!r}, candidate={actual!r}", file=sys.stderr)
            return 1
        print(f"{name}: match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
