#!/usr/bin/env python3
"""Paired checked table benchmark (one million rows is explicitly opt-in)."""
import argparse
import json
from pathlib import Path
import platform
import re
from tempfile import TemporaryDirectory

from machina import Arm, rss_tool, run_workload, emit, sha256


def scripts(n):
    common = f'<< "df";\nxs := .. 1z {n}z;\n'
    columns = 't := #: .{ first := xs; second := xs; third := xs; };\n'
    pick = 'pick(r:.{ first:z; second:z; third:z; }) := .{ c := .third r; a := .first r; };\n'
    yield emit(f"wide-setup-{n}", "tabula", common + columns, "",
               ["# t", "(?! (.: 1n .first t))", f"(?! (.: {n}n .third t))"],
               [n, 1, n], rows=n, columns=3)
    yield emit(f"project-rename-{n}", "tabula", common + columns + pick,
               "p := df.select &pick t;",
               ["# p", "(?! (.: 1n .c p))", f"(?! (.: {n}n .a p))"],
               [n, 1, n], rows=n, columns=2)
    m = min(n, 8192)
    group = (
        f'<< "df";\nxs := .. 1z {m}z;\n'
        't := #: .{ bucket := % xs 17z; item := xs; };\n'
        'summ(k:z,rows:#.{ bucket:z; item:z; }) := .{ bucket := k; count := # rows; total := +/ .item rows; };'
    )
    yield emit(f"grouped-sum-{m}", "tabula", group, "out := df.agg &.bucket &summ t;",
               ["# out", "+/ .count out", "+/ .total out"],
               [min(m, 17), m, m * (m + 1) // 2], rows=m, groups=min(m, 17))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("before", type=Path)
    p.add_argument("after", type=Path)
    p.add_argument("--rows", type=int, default=8192)
    p.add_argument("--large", action="store_true")
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--match", default=".")
    p.add_argument("--generic", action="store_true")
    p.add_argument("--phases", action="store_true")
    p.add_argument("--no-rss", action="store_true")
    p.add_argument("--timeout", type=float, default=120)
    p.add_argument("--max-elements", type=int)
    p.add_argument("--max-memory", type=int)
    p.add_argument("--max-work", type=int)
    args = p.parse_args()
    if not 1 <= args.rows <= 1_000_000 or args.runs < 3 or not args.timeout > 0:
        p.error("rows must be 1–1,000,000, runs at least 3, timeout positive")
    if args.rows > 100_000 and (not args.large or not all(
        x and x > 0 for x in (args.max_elements, args.max_memory, args.max_work)
    )):
        p.error("large runs require --large and explicit --max-elements, --max-memory and --max-work")
    args.allow_limits = args.allow_timeouts = False
    limits = {
        "FLUX_MAX_" + key.upper(): str(getattr(args, "max_" + key))
        for key in ("elements", "memory", "work") if getattr(args, "max_" + key) is not None
    }
    before, after = (b.resolve(strict=True) for b in (args.before, args.after))
    arms = [Arm("before", before, (), {**limits, "FLUX_OPTIMIZE": "1"}),
            Arm("after", after, (), {**limits, "FLUX_OPTIMIZE": "1"})]
    if args.generic:
        arms.append(Arm("after-generic", after, (), {**limits, "FLUX_OPTIMIZE": "0"}))
    matcher = re.compile(args.match)
    timer = rss_tool(args.no_rss)
    with TemporaryDirectory(prefix="flux-tabula-") as temp:
        directory = Path(temp)
        source = directory / "tabula.flx"
        result = {
            w.name: run_workload(w, arms, directory, source, timer, args)
            for w in scripts(args.rows) if matcher.search(w.name)
        }
    if not result:
        p.error("no matched benchmark workloads")
    print(json.dumps({
        "platform": platform.platform(), "rows": args.rows, "rss": timer,
        "phase_scope": "separate checked CLI timings; eval includes construction and I/O",
        "arms": {arm.name: {"binary": str(arm.binary), "sha256": sha256(arm.binary),
                            "env": arm.env} for arm in arms},
        "results": result,
    }, indent=2))


if __name__ == "__main__":
    main()
