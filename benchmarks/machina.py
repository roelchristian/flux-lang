#!/usr/bin/env python3
"""Paired Flux end-to-end benchmarks with checked results and optional Linux peak RSS.

Use locked release binaries. Example (small 0.17-only smoke run):
  python3 benchmarks/machina.py OLD OLD --groups shared dense matrix --runs 3

Compare construction and in-process phases (Draft 0.20 and later):
  python3 benchmarks/machina.py FLUX_020 FLUX_021 --generic --phases --groups construction dense edge --elements 10000 100000 --runs 5

Wall time includes startup, checking, preparation and checked output.
Phase eval time includes preparation and output; it is not kernel-only.
Matched setup controls and trials at different --kernel-repeats help
distinguish those costs. Large vectors and 128–1024 square products are
opt-in. Use explicit per-arm FLUX_MAX_ELEMENTS, FLUX_MAX_WORK and
FLUX_MAX_MEMORY overrides for 10M; inherited FLUX_* settings are removed.
"""
import argparse
import csv
from dataclasses import dataclass, field
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
from statistics import median
import subprocess
import sys
from tempfile import TemporaryDirectory
from time import perf_counter_ns


DEFAULT_GROUPS = ("shared", "dense", "mathesis", "matrix")
GROUPS = (*DEFAULT_GROUPS, "construction", "edge")
PHASE_KEYS = ("parse_ns", "resolve_ns", "infer_ns", "elaborate_ns", "eval_ns")
MATRIX_SIZES = (128, 256, 512, 1024)
LARGE_SIZES = (100_000, 1_000_000, 10_000_000)
INT_LIMIT = 10_000_000
WARMUPS = 1

# Match only runtime budget diagnostics, not type/parser errors or arbitrary
# crashes. In particular, a killed or panicking process is never a limit pass.
LIMIT_DIAGNOSTIC = re.compile(
    r"resource limit|array dimension exceeds|array element count exceeds|"
    r"runtime numeric iteration limit|prototype nesting or recursion limit|"
    r"collection limit|prototype range limit|(?:array |logical )?element (?:limit|budget)|"
    r"(?:tracked |working )?memory (?:limit|budget)|"
    r"(?:work|execution|runtime|evaluator|allocation|evaluation|fuel) "
    r"(?:limit|budget)(?: exhausted| exceeded)?|"
    r"(?:too many|excessive) (?:logical |array )?elements",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class Workload:
    name: str
    group: str
    source: str
    expected: str
    mode: str = "run-hir"
    metadata: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Arm:
    name: str
    binary: Path
    flags: tuple
    env: dict


def sequence(items):
    return "[" + " ".join(map(str, items)) + "]"


def dimensions(*shape):
    return sequence(f"{n}n" for n in shape)


def result(*items):
    return "(" + ", ".join(map(str, items)) + ")"


def script(definitions, body, prints=1):
    return (
        definitions
        + "\n!^{\n"
        + ("    ! print " + body + ";\n") * prints
        + "};\n"
    )


def sampled(length):
    return tuple(dict.fromkeys((1, (length + 1) // 2, length)))


def cell(name, *indices):
    return f"(?! (.: {dimensions(*indices)} {name}))"


def numeric_sample(name, coords):
    return [cell(name, *indices) for indices in coords]


def real_literal(value):
    return repr(float(value)) + "r"


def approximate_sample(name, coords, want):
    return [
        f"(< (| (- {cell(name, *point)} {real_literal(expected)})) 1e-12r)"
        for point, expected in zip(coords, want)
    ]


def emit(name, group, definitions, steps, expressions, answers, *,
         repeat=1, **metadata):
    """Print only a small verification payload even for millions of cells."""
    if len(expressions) != len(answers):
        raise ValueError("workload verification width disagrees")
    body = ("{ " + steps + " " if steps else "") + result(*expressions)
    if steps:
        body += " }"
    return Workload(
        name,
        group,
        script(definitions, body, repeat),
        (result(*answers) + "\n") * repeat,
        metadata={"kernel_repeats": repeat, **metadata},
    )


def shared_workloads(args, directory):
    n = args.scalar_elements
    yield Workload(
        f"squares-pipeline-{n}",
        "shared",
        script("", f"|{{ .. 1z {n}z; ^ $ 2n; +/ $ }}"),
        f"{n * (n + 1) * (2 * n + 1) // 6}\n",
        metadata={"elements": n},
    )
    decls = args.declarations
    yield Workload(
        f"scalar-check-{decls}",
        "shared",
        "".join(f"x{i} := + {i}z (* {i}z 2z);\n" for i in range(decls)),
        "",
        mode="check",
        metadata={"declarations": decls},
    )
    n, groups = args.csv_rows, min(args.csv_rows, 32)
    with (directory / "observations.csv").open("w", encoding="ascii", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(("value", "bucket"))
        writer.writerows((i, f"k{i % groups}") for i in range(n))
    yield Workload(
        f"csv-aggregate-{n}",
        "shared",
        f'''<< "csv"; << "df";
obs :: {{ bucket:s; value:z; }};
summ(k:s,t:#obs) := .{{ bucket := k; total := +/ .value t; }};
none:[obs] := [];
!^{{
    t:#obs := ^= ! csv.read "observations.csv" {{ _ : #: none; }};
    g := df.agg &.bucket &summ t;
    ! print (# g, +/ .total g);
}};
''',
        result(groups, n * (n - 1) // 2) + "\n",
        metadata={"rows": n, "groups": groups},
    )


def real_vector(n):
    return f"a := / (:r (form {dimensions(n)} (.. 1z {n}z))) {n}r;"


def real_output(name, group, definitions, steps, points, values, n, repeat=1):
    return emit(
        name, group, definitions, steps,
        ["dim value", *approximate_sample("value", points, values)],
        [sequence([n]), *("1b" for _ in points)],
        repeat=repeat, elements=n, input_payload_f64_bytes=8 * n,
    )


def dense_workloads(args, n):
    shape = dimensions(n)
    definitions = f"a := form {shape} (.. 1z {n}z);"
    points = [(i,) for i in sampled(n)]
    expected = [i for i, in points]
    yield emit(
        f"dense-input-{n}", "dense", definitions, "",
        ["dim a", *numeric_sample("a", points)],
        [sequence([n]), *expected],
        elements=n,
    )
    yield emit(
        f"dense-arithmetic-{n}", "dense", definitions,
        "value := + (* a 3z) 2z;",
        ["dim value", *numeric_sample("value", points)],
        [sequence([n]), *(i * 3 + 2 for i in expected)],
        repeat=args.kernel_repeats, elements=n,
    )
    yield emit(
        f"dense-sum-{n}", "dense", definitions,
        "value := +/ a;",
        ["dim value", cell("value")], ["[]", n * (n + 1) // 2],
        repeat=args.kernel_repeats, elements=n,
    )
    # Keep these contiguous real inputs integer-valued. The binary64
    # products and every sum prefix stay below 2**53 even at 10M cells,
    # so Python's integer formulas are independent, exact scalar oracles.
    real = f"a := :r (form {shape} (.. 1z {n}z));"
    real_metadata = {"elements": n, "input_payload_f64_bytes": 8 * n}
    yield emit(
        f"real-input-{n}", "dense", real, "",
        ["dim a", *numeric_sample("a", points)], [sequence([n]), *expected],
        **real_metadata,
    )
    yield emit(
        f"real-arithmetic-{n}", "dense", real, "value := + (* a 3r) 2r;",
        ["dim value", *numeric_sample("value", points)],
        [sequence([n]), *(i * 3 + 2 for i in expected)],
        repeat=args.kernel_repeats, **real_metadata,
    )
    yield emit(
        f"real-sum-{n}", "dense", real, "value := +/ a;",
        ["dim value", cell("value")], ["[]", n * (n + 1) // 2],
        repeat=args.kernel_repeats, **real_metadata,
    )
    yield real_output(
        f"real-negation-chain-{n}", "dense", real,
        "value := -: -: -: -: -: a;", points, [-i for i in expected], n,
        args.kernel_repeats,
    )
    yield emit(
        f"real-comparison-{n}", "dense", real,
        f"value := >= a {real_literal(n / 2)};",
        ["dim value", *numeric_sample("value", points)],
        [sequence([n]), *("1b" if 2 * i >= n else "0b" for i in expected)],
        repeat=args.kernel_repeats, **real_metadata,
    )
    rows, cols = rectangular_shape(n, args.width)
    count = n
    definitions = f"a := form {dimensions(rows, cols)} (.. 1z {count}z);"
    points = list(zip(sampled(rows), sampled(cols)))
    points.extend(((1, cols), (rows, 1)))
    points = list(dict.fromkeys(points))
    yield emit(
        f"dense-broadcast-{rows}x{cols}", "dense",
        definitions + f"\nb := form {dimensions(cols)} (.. 101z {100 + cols}z);",
        "value := + a b;",
        ["dim value", *numeric_sample("value", points)],
        [sequence([rows, cols]), *((r - 1) * cols + 2 * c + 100 for r, c in points)],
        repeat=args.kernel_repeats, elements=count,
    )
    # Reverse both axes, then take a nonzero-offset rectangular view.
    starts = (2 if cols > 1 else 1, 2 if rows > 1 else 1)
    c_count, r_count = cols - starts[0] + 1, rows - starts[1] + 1
    points = list(dict.fromkeys(((1, 1), (c_count, 1), (1, r_count), (c_count, r_count))))
    yield emit(
        f"dense-transpose-section-{rows}x{cols}", "dense",
        '<< "arr";\n' + definitions,
        f"value := arr.sect {dimensions(*starts)} {dimensions(c_count, r_count)} (|: a);",
        ["dim value", *numeric_sample("value", points)],
        [sequence([c_count, r_count]),
         *((r + starts[1] - 2) * cols + c + starts[0] - 1 for c, r in points)],
        repeat=args.kernel_repeats, elements=count,
    )


def rectangular_shape(n, width):
    cols = next(c for c in range(min(n, width), 0, -1) if n % c == 0)
    return n // cols, cols


def construction_workloads(args, n):
    """Control input paths without hiding escaping sequences or strided copies."""
    points = sampled(n)
    coords = [(i,) for i in points]
    items = f".. 1z {n}z"
    shape = dimensions(n)
    yield emit(
        f"construction-range-{n}", "construction", f"a := {items};", "",
        ["# a", *(f"(?! (.: {i}n a))" for i in points)],
        [n, *points], elements=n, path="ordinary range",
    )
    # The sequence remains independently observable after it is used by form.
    yield emit(
        f"construction-escaped-form-{n}", "construction",
        f"items := {items};\na := form {shape} items;", "",
        ["dim a", "# items", *numeric_sample("a", coords)],
        [sequence([n]), n, *points], elements=n, path="escaped sequence",
    )
    for domain in ("z", "n", "r"):
        init = (f":r (form {shape} ({items}))" if domain == "r"
                else f"form {shape} (.. 1{domain} {n}{domain})")
        yield emit(
            f"construction-inline-{domain}-{n}", "construction", f"a := {init};", "",
            ["dim a", *numeric_sample("a", coords)],
            [sequence([n]), *points], elements=n, path=f"inline {domain}",
        )
    # This control matches the input preparation of the Mathesis workloads.
    yield real_output(
        f"construction-normalized-real-{n}", "construction",
        '<< "math";\n' + real_vector(n), "value := a;",
        coords, [i / n for i in points], n,
    )

    rows, cols = rectangular_shape(n, args.width)
    base = f"a := :r (form {dimensions(rows, cols)} ({items}));"
    for layout, source in (("contiguous", "a"), ("strided", "(|: a)")):
        wanted = points if layout == "contiguous" else tuple(
            ((i - 1) % rows) * cols + (i - 1) // rows + 1 for i in points
        )
        yield emit(
            f"construction-reshape-{layout}-{n}", "construction", base,
            f"value := form {shape} {source};",
            ["dim value", *numeric_sample("value", coords)],
            [sequence([n]), *wanted], repeat=args.kernel_repeats,
            elements=n, path=f"{layout} reshape",
        )

    # Combine a strided input with a singleton-axis broadcast.
    view_points = list(dict.fromkeys(((1, 1), (cols, rows), ((cols + 1) // 2, (rows + 1) // 2))))
    values = [((j - 1) * cols + i) + j for i, j in view_points]
    yield emit(
        f"real-strided-broadcast-{n}", "construction",
        base + f"\nb := |: a;\noffsets := :r (form {dimensions(rows)} (.. 1z {rows}z));",
        "value := + b offsets;",
        ["dim value", *numeric_sample("value", view_points)],
        [sequence([cols, rows]), *values], repeat=args.kernel_repeats, elements=n,
    )


def edge_workloads(args, n):
    # Every element grows past compact i64; never measure wrapping as success.
    end = (1 << 63) - 1
    points = sampled(n)
    yield emit(
        f"exact-widening-{n}", "edge",
        f"a := form {dimensions(n)} (.. {end - n + 1}z {end}z);",
        f"value := + a {n}z;",
        ["dim value", *numeric_sample("value", [(i,) for i in points])],
        [sequence([n]), *(end + i for i in points)], repeat=args.kernel_repeats,
        elements=n,
    )
    # Keep arbitrary precision separate from large compact throughput.
    count = min(n, 2048)
    big = 1 << 128
    points = sampled(count)
    yield emit(
        f"arbitrary-exact-{count}-at-{n}", "edge",
        f"a := form {dimensions(count)} (.. {big + 1}z {big + count}z);",
        "value := + a 3z;",
        ["dim value", *numeric_sample("value", [(i,) for i in points])],
        [sequence([count]), *(big + i + 3 for i in points)],
        repeat=args.kernel_repeats, elements=count,
    )
    columns = max(n // 2, 1)
    definitions = (
        f"positive := :r (form {dimensions(columns)} (.. 1z {columns}z));\n"
        "pattern := form [2n 1n] [-0.0r 0.0r];\na := * pattern positive;"
    )
    checks = [(1, 1), (2, columns)]
    for parity, chain, values in (
        ("even", "-: -: -: -: a", ("-0", "0")),
        ("odd", "-: -: -: -: -: a", ("0", "-0")),
    ):
        yield emit(
            f"signed-zero-{parity}-{n}", "edge", definitions, f"value := {chain};",
            ["dim value", *numeric_sample("value", checks)],
            [sequence([2, columns]), *values],
            repeat=args.kernel_repeats, elements=2 * columns,
        )


def mathesis_workloads(args, n):
    points = [(i,) for i in sampled(n)]
    definitions = '<< "math";\n' + real_vector(n)
    for function in ("sin", "exp"):
        values = [getattr(math, function)(i / n) for i, in points]
        yield real_output(
            f"dense-{function}-{n}", "mathesis", definitions,
            f"value := math.{function} a;", points, values, n, args.kernel_repeats,
        )


def matrix_workloads(args, n):
    # A is row-major 1..n², B is A's transpose. Every coefficient/product
    # and partial sum is exactly representable in binary64 for n <= 1024.
    # Samples have a closed-form dot product, independent of Flux kernels.
    definitions = (
        '<< "arr";\n'
        f"a := :r (form {dimensions(n, n)} (.. 1z {n * n}z));\n"
    )
    points = list(dict.fromkeys(((1, 1), (1, n), ((n + 1) // 2, (n + 1) // 2), (n, n))))
    sum_k = n * (n + 1) // 2
    sum_k2 = n * (n + 1) * (2 * n + 1) // 6
    want = [
        n ** 3 * (i - 1) * (j - 1) + n * (i + j - 2) * sum_k + sum_k2
        for i, j in points
    ]
    for layout, binding in (
        ("strided", "b := |: a;"),
        ("contiguous", f"b := form {dimensions(n, n)} (,. (|: a));"),
    ):
        setup = definitions + binding
        metadata = {"elements": n * n, "right_layout": layout,
                    "input_payload_f64_bytes": 8 * n * n}
        shape = sequence([n, n])
        yield emit(
            f"matrix-input-{layout}-{n}x{n}", "matrix", setup, "",
            ["dim b", *numeric_sample("b", points)],
            [shape, *((j - 1) * n + i for i, j in points)],
            **metadata,
        )
        yield emit(
            f"matmul-{layout}-{n}x{n}", "matrix", setup, "value := arr.mul a b;",
            ["dim value", *numeric_sample("value", points)], [shape, *want],
            repeat=args.kernel_repeats, multiply_accumulates=n ** 3, **metadata,
        )


def workloads(args, directory):
    if "shared" in args.groups:
        yield from shared_workloads(args, directory)
    sizes = list(dict.fromkeys(args.elements + (list(LARGE_SIZES) if args.large else [])))
    for n in sizes:
        if "dense" in args.groups:
            yield from dense_workloads(args, n)
        if "mathesis" in args.groups:
            yield from mathesis_workloads(args, n)
        if "construction" in args.groups:
            yield from construction_workloads(args, n)
        if "edge" in args.groups:
            yield from edge_workloads(args, n)
    if "matrix" in args.groups:
        sizes = list(MATRIX_SIZES) if args.standard_matrices else args.matrix_sizes
        for n in dict.fromkeys(sizes):
            yield from matrix_workloads(args, n)


def env_pairs(items):
    pairs = {}
    for pair in items:
        key, sep, value = pair.partition("=")
        if not sep or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key) or "\0" in value:
            raise ValueError(f"invalid KEY=VALUE override: {pair!r}")
        pairs[key] = value
    return pairs


def arm(name, path, flags, env):
    overrides = env_pairs(env)
    # Explicit options and legacy CLI prefixes may choose another policy.
    if not flags:
        overrides.setdefault("FLUX_OPTIMIZE", "1")
    return Arm(name, path.resolve(strict=True), tuple(flags), overrides)


def process_env(overrides):
    return {
        **{name: value for name, value in os.environ.items()
           if not name.upper().startswith("FLUX_")},
        **overrides,
    }


def phase_metrics(stderr):
    """Accept only the timing protocol from a successful phase invocation."""
    timings = {}
    for line in stderr.splitlines():
        key, sep, value = line.partition("\t")
        if not sep or key not in PHASE_KEYS or key in timings or not value.isdecimal():
            return None
        timings[key] = int(value)
    return timings if len(timings) == len(PHASE_KEYS) else None


def resource_failure(stderr):
    # Do not let code excerpts/comments relabel ordinary fatal errors.
    lines = stderr.splitlines()
    if not lines:
        return False
    primary = re.match(r"^(?:.*?: )?R0001: (.*)$", lines[0])
    return bool(primary and LIMIT_DIAGNOSTIC.search(primary.group(1)))


# Run one Flux child per wrapper, so Linux RUSAGE_CHILDREN is per run
# rather than a cumulative maximum across samples. Exclude Python's wrapper
# startup from wall time. GNU time is preferred if present.
RUSAGE_WRAPPER = """
import resource, subprocess, sys, time
from pathlib import Path
start = time.perf_counter_ns()
child = subprocess.run(sys.argv[2:])
elapsed = time.perf_counter_ns() - start
rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
Path(sys.argv[1]).write_text(f"{rss} {elapsed}", encoding="ascii")
if child.returncode < 0:
    raise SystemExit(128 - child.returncode)
raise SystemExit(child.returncode)
"""


def rss_tool(disabled):
    if disabled or sys.platform != "linux":
        return None
    path = Path("/usr/bin/time")
    if path.is_file():
        try:
            info = subprocess.run([str(path), "--version"], capture_output=True, timeout=3)
            if b"GNU" in info.stdout + info.stderr:
                return str(path)
        except (OSError, subprocess.TimeoutExpired):
            pass
    return "linux-rusage"


def timed(arm, workload, directory, source, timer, args, *, phase=False):
    cmd = [str(arm.binary), *arm.flags, "phases" if phase else workload.mode, source.name]
    rss_path = directory / "peak-rss.txt"
    if timer:
        rss_path.unlink(missing_ok=True)
    if timer == "linux-rusage":
        cmd = [sys.executable, "-c", RUSAGE_WRAPPER, str(rss_path), *cmd]
    elif timer:
        cmd = [timer, "-q", "-f", "%M", "-o", str(rss_path), "--", *cmd]
    begin = perf_counter_ns()
    with subprocess.Popen(
        cmd, cwd=directory, env=process_env(arm.env),
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, encoding="utf-8", start_new_session=timer is not None,
    ) as process:
        try:
            stdout, stderr = process.communicate(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            # Killing the RSS wrapper alone leaves its Flux child running.
            if timer:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            else:
                process.kill()
            process.communicate()
            if args.allow_timeouts:
                return {"status": "timeout", "seconds": args.timeout}
            raise
    wall_ms = (perf_counter_ns() - begin) / 1e6
    phases = phase_metrics(stderr) if phase else None
    invalid_diagnostics = phases is None if phase else bool(stderr)
    if process.returncode or stdout != workload.expected or invalid_diagnostics:
        if (
            args.allow_limits
            and process.returncode == 1
            and (not stdout or stdout.endswith("\n"))
            and workload.expected.startswith(stdout)
            and "panic" not in stderr.lower()
            and resource_failure(stderr)
        ):
            return {"status": "limited", "stderr": stderr.strip(),
                    "verified_output_lines": len(stdout.splitlines())}
        raise RuntimeError(
            f"{arm.name} {workload.name}: exit {process.returncode}\n"
            f"expected {workload.expected!r}; got {stdout!r}\n{stderr}"
        )
    peak_rss = None
    if timer and rss_path.exists():
        data = rss_path.read_text(encoding="ascii").split()
        if timer == "linux-rusage" and len(data) == 2:
            peak_rss, elapsed_ns = map(int, data)
            wall_ms = elapsed_ns / 1e6
        elif len(data) == 1 and data[0].isdigit():
            peak_rss = int(data[0])
    measured = {"status": "ok", "wall_ms": wall_ms, "peak_rss_kib": peak_rss}
    if phases is not None:
        measured["phase_ns"] = phases
    return measured


def summarize(samples):
    walls = [v["wall_ms"] for v in samples]
    rss = [v["peak_rss_kib"] for v in samples if v["peak_rss_kib"] is not None]
    result = {
        "status": "ok",
        "wall_median_ms": round(median(walls), 3),
        "wall_samples_ms": [round(x, 3) for x in walls],
        "rss_median_kib": median(rss) if rss else None,
        "rss_samples_kib": [v["peak_rss_kib"] for v in samples],
    }
    if "phase_ns" in samples[0]:
        phases = {key: [v["phase_ns"][key] for v in samples] for key in PHASE_KEYS}
        result["phase_samples_ns"] = phases
        result["phase_medians_ns"] = {key: round(median(values)) for key, values in phases.items()}
    return result


def run_workload(workload, arms, directory, source, timer, args, *, phase=False):
    source.write_text(workload.source, encoding="ascii")
    samples = {item.name: [] for item in arms}
    results = {}
    active = []
    for item in arms:
        if item.name == "reference-0.17" and workload.group == "mathesis":
            results[item.name] = {"status": "skipped", "reason": "not supported in Draft 0.17"}
            continue
        check = timed(item, workload, directory, source, timer, args, phase=phase)
        if check["status"] == "ok":
            active.append(item)
        else:
            results[item.name] = check
    # Warmup is also verified. Rotate/reverse, rather than always reheating
    # or cooling one binary first when there are more than two arms.
    for run in range(args.runs):
        order = active[run % len(active):] + active[:run % len(active)] if active else []
        if (run // max(len(active), 1)) % 2:
            order.reverse()
        for item in order:
            if item.name in results:
                continue
            data = timed(item, workload, directory, source, timer, args, phase=phase)
            if data["status"] != "ok":
                results[item.name] = {**data, "successful_samples": len(samples[item.name])}
            else:
                samples[item.name].append(data)
    for name, observations in samples.items():
        if name not in results:
            results[name] = summarize(observations)
    result = {
        "group": workload.group,
        "source_sha256": hashlib.sha256(workload.source.encode("ascii")).hexdigest(),
        **workload.metadata,
        "arms": results,
    }
    if getattr(args, "phases", False) and not phase:
        # The phases CLI does not propagate a natural script exit status.
        # Verify ordinary execution before running it for timing.
        checked = [item for item in arms if results[item.name]["status"] == "ok"]
        if checked:
            phase_arms = run_workload(workload, checked, directory, source, None, args, phase=True)
            result["phase_arms"] = phase_arms["arms"]
    return result


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for data in iter(lambda: file.read(64 * 1024), b""):
            digest.update(data)
    return digest.hexdigest()


def bound(value, lo, hi, label):
    if not lo <= value <= hi:
        raise ValueError(f"{label} must be between {lo} and {hi}")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path, help="baseline release binary (e.g. locked Draft 0.20 build)")
    parser.add_argument("after", type=Path, help="candidate release binary")
    parser.add_argument("--reference", type=Path, help="optional Draft 0.17 binary, shared subset")
    parser.add_argument("--generic", action="store_true",
                        help="also test the candidate with FLUX_OPTIMIZE=0 (or an explicit legacy flag)")
    for prefix in ("before", "after", "reference", "generic"):
        parser.add_argument(f"--{prefix}-arg", action="append", default=[],
                            help="repeatable CLI prefix argument; use = for flags")
        parser.add_argument(f"--{prefix}-env", action="append", default=[], metavar="KEY=VALUE",
                            help="repeatable subprocess environment override")
    parser.add_argument("--groups", nargs="+", choices=GROUPS, default=list(DEFAULT_GROUPS))
    parser.add_argument("--match", action="append", default=[], metavar="REGEX",
                        help="only workload names matching at least one pattern")
    parser.add_argument("--elements", nargs="+", type=int, default=[10_000],
                        help="dense/vector element counts, at most 100,000 without --large")
    parser.add_argument("--large", action="store_true",
                        help="opt in to additionally testing 100,000, 1M and 10M elements")
    parser.add_argument("--matrix-sizes", nargs="+", type=int, default=[16, 32])
    parser.add_argument("--standard-matrices", action="store_true",
                        help="use 128, 256, 512, 1024 instead of --matrix-sizes")
    parser.add_argument("--scalar-elements", type=int, default=10_000)
    parser.add_argument("--declarations", type=int, default=4_000)
    parser.add_argument("--csv-rows", type=int, default=6_000)
    parser.add_argument("--width", type=int, default=64, help="trailing axis of rectangular workloads")
    parser.add_argument("--kernel-repeats", type=int, default=1,
                        help="repeat each kernel on shared inputs within each timed script")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=120.0, help="seconds per subprocess")
    parser.add_argument("--phases", action="store_true",
                        help="also collect paired CLI phase timings on verified arms")
    parser.add_argument("--no-rss", action="store_true", help="disable peak RSS sampling")
    parser.add_argument("--allow-limits", action="store_true",
                        help="report checked budget diagnostics as unmeasured limits, not successes")
    parser.add_argument("--allow-timeouts", action="store_true",
                        help="report timeouts as unmeasured arms instead of aborting")
    args = parser.parse_args()
    try:
        if args.runs < 3 or not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("runs must be at least 3; timeout must be positive and finite")
        for n in args.elements:
            bound(n, 1, INT_LIMIT if args.large else 100_000, "elements")
        for n in args.matrix_sizes:
            bound(n, 1, 1024, "matrix size")
        for n in (args.scalar_elements, args.declarations, args.csv_rows, args.width):
            bound(n, 1, 100_000, "shared/width size")
        bound(args.kernel_repeats, 1, 64, "kernel repeats")
        arms = [arm("before", args.before, args.before_arg, args.before_env),
                arm("after", args.after, args.after_arg, args.after_env)]
        if args.reference:
            arms.insert(0, arm("reference-0.17", args.reference, args.reference_arg, args.reference_env))
        if args.generic:
            after = arms[-1]
            overrides = {k: v for k, v in after.env.items() if k != "FLUX_OPTIMIZE"}
            generic = env_pairs(args.generic_env)
            if generic.get("FLUX_OPTIMIZE", "0") != "0":
                raise ValueError("--generic requires FLUX_OPTIMIZE=0")
            if not args.generic_arg:
                generic.setdefault("FLUX_OPTIMIZE", "0")
            arms.append(Arm("after-generic", after.binary, after.flags + tuple(args.generic_arg),
                            {**overrides, **generic}))
        matches = tuple(map(re.compile, args.match))
    except (OSError, ValueError, re.error) as exc:
        parser.error(str(exc))
    timer = rss_tool(args.no_rss)
    with TemporaryDirectory(prefix="flux-machina-") as temp:
        directory = Path(temp)
        source = directory / "machina.flx"
        results = {
            w.name: run_workload(w, arms, directory, source, timer, args)
            for w in workloads(args, directory)
            if not matches or any(regex.search(w.name) for regex in matches)
        }
    if not results:
        parser.error("no workloads matched")
    rss_label = {
        None: "unavailable or disabled",
        "linux-rusage": "Linux RUSAGE_CHILDREN peak resident KiB",
    }.get(timer, "GNU time peak resident KiB")
    print(json.dumps({
        "platform": platform.platform(),
        "python": platform.python_version(),
        "rss": rss_label,
        "wall": "process startup, checking, setup, kernels and small checked output",
        "phase_scope": ("separate CLI samples; eval includes setup and output"
                        if args.phases else None),
        "policy": "inherited FLUX_* removed; arm env lists overrides; otherwise binary defaults",
        "runs": args.runs,
        "warmups": WARMUPS,
        "argv": sys.argv[1:],
        "arms": {
            item.name: {"binary": str(item.binary), "sha256": sha256(item.binary),
                        "flags": item.flags, "env": item.env}
            for item in arms
        },
        "results": results,
    }, indent=2))


if __name__ == "__main__":
    main()
