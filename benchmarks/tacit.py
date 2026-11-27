#!/usr/bin/env python3
"""Compare 0.7 tacit expressions with their ordinary closure equivalents."""

import argparse
from pathlib import Path
from statistics import median
from subprocess import run
from tempfile import TemporaryDirectory
from time import perf_counter_ns


def fixture(prefix, tacit, explicit, body, expected, command="run-hir"):
    return (
        prefix + "\n" + tacit + "\n" + body,
        prefix + "\n" + explicit + "\n" + body,
        expected,
        command,
    )


EMIT = '!{{ ^{{ ^? ! print ({expression}); () }} }};\n'


def cases():
    n = 12_000
    yield "scalar composition", fixture(
        f"bump(x:z):z := + x 1z;\ndouble(x:z):z := * x 2z;\na := .. 1z {n}z;",
        "f:z->z := &{ @. double bump };",
        r"f:z->z := \x -> @ &double (@ &bump x);",
        EMIT.format(expression="+/ (@: f a)"),
        f"{n * (n + 3)}\n",
    )
    yield "square / map / reduce", fixture(
        f"a := .. 1z {n}z;",
        "sq:z->z := &{ @* * . . };",
        r"sq:z->z := \x -> @@ &* (x,x);",
        EMIT.format(expression="+/ (@: sq a)"),
        f"{n * (n + 1) * (2 * n + 1) // 6}\n",
    )
    rows = 6000
    yield "financial table projection", fixture(
        f'''<< "df";
sale :: {{ id:z; revenue:d; cost:d; tax:d; }};
mk(x:z):sale := sale {{ id:=x; revenue:=125.50d; cost:=80.25d; tax:=5.00d; }};
sales:#sale := #: (@: &mk (.. 1z {rows}z));''',
        "gain:sale->d := &{ @* - .revenue (@* + .cost .tax) };",
        r"gain:sale->d := \x -> @@ &- (@ &.revenue x,@@ &+ (@ &.cost x,@ &.tax x));",
        r'''out := df.select (\x -> .{ id:=.id x; gain:=@ gain x; }) sales;
''' + EMIT.format(expression="+/ (.gain out)"),
        f"{rows * 4025 // 100}.00\n",
    )
    half = rows // 2
    yield "relational transformed keys", fixture(
        f'''<< "df";
ledger :: {{ seq:z; code:s; }};
account(x:z):ledger := ledger {{ seq:=x; code:="  shared  "; }};
left:#ledger := #: (@: &account (.. 1z {rows}z));
right:#ledger := #: (@: &account (.. {half + 1}z {rows + half}z));
<< "str";
key2(a:z,b:s):(z,s) := (a,b);''',
        "key:ledger->(z,s) := &{ @* key2 .seq (@. str.trim .code) };",
        r"key:ledger->(z,s) := \x -> @@ &key2 (@ &.seq x,@ &str.trim (@ &.code x));",
        EMIT.format(expression="# (*= key key left right), # (df.anti key key left right)"),
        f"({rows-half}, {half})\n",
    )
    depth = 28
    tacit = "@. bump " * depth + "."
    explicit = "(" + "@ &bump (" * depth + "x" + ")" * (depth + 1)
    yield "compile nested composition", fixture(
        "bump(x:z):z := + x 1z;",
        f"f:z->z := &{{ {tacit} }};",
        rf"f:z->z := \x -> {explicit};",
        EMIT.format(expression="@ f 1z"),
        f"{depth+1}\n",
        "check",
    )


def invoke(binary, command, script, expected):
    start = perf_counter_ns()
    result = run([binary, command, script], capture_output=True)
    ms = (perf_counter_ns() - start) / 1_000_000
    if result.returncode or result.stderr or result.stdout != expected.encode("ascii"):
        raise RuntimeError(
            f"{script} {command}, status={result.returncode}: "
            f"expected={expected!r}, stdout={result.stdout!r}, stderr={result.stderr!r}"
        )
    return ms


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("binary", type=Path, help="typed Flux release CLI with Draft 0.7 syntax")
    cli.add_argument("--runs", type=int, default=7, help="measured pairs, excluding warmup")
    args = cli.parse_args()
    if args.runs < 3:
        cli.error("at least three measured run pairs are required")
    binary = args.binary.resolve(strict=True)
    with TemporaryDirectory(prefix="flux-tacit-") as directory:
        paths = [Path(directory) / name for name in ("tacit.flx", "explicit.flx")]
        for name, (tacit, explicit, output, command) in cases():
            for path, source in zip(paths, (tacit, explicit)):
                path.write_text(source, encoding="ascii")
                # Check exact equivalent output even for the compiler-only benchmark.
                invoke(binary, "run-hir", path, output)
            samples = [[], []]
            for i in range(args.runs):
                for variant in (i % 2, 1 - i % 2):
                    measured_output = "" if command == "check" else output
                    samples[variant].append(invoke(binary, command, paths[variant], measured_output))
            tacit_ms, explicit_ms = map(median, samples)
            print(
                f"{name:28s} tacit_ms={tacit_ms:8.3f} "
                f"explicit_ms={explicit_ms:8.3f} ratio={tacit_ms / explicit_ms:.3f}"
            )


if __name__ == "__main__":
    main()
