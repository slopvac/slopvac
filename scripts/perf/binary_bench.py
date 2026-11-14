#!/usr/bin/env python3
"""Measure an installed slopvac binary: startup, lint time, peak RSS, size.

Stdlib only. It reports; it does not gate unless --enforce is passed with a
budgets file. Run `binary_bench.py run --binary PATH`, then compare two JSON
results with `binary_bench.py compare BASE HEAD`.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
SCENARIOS = ROOT / "scripts/perf/binary-scenarios.toml"
SCHEMA = 1
WARMUP = 3
RUNS = 20


def percentile(values: list[float], fraction: float) -> float:
    """Nearest-rank percentile of a non-empty list."""
    ordered = sorted(values)
    rank = max(1, math.ceil(fraction * len(ordered)))
    return ordered[rank - 1]


def rss_kb(raw: int) -> int:
    """ru_maxrss is bytes on macOS and kilobytes elsewhere."""
    return raw // 1024 if sys.platform == "darwin" else raw


def summarize(times_ms: list[float], rss: list[int]) -> dict[str, Any]:
    return {
        "median_ms": round(statistics.median(times_ms), 3),
        "p95_ms": round(percentile(times_ms, 0.95), 3),
        "min_ms": round(min(times_ms), 3),
        "max_ms": round(max(times_ms), 3),
        "peak_rss_kb": max(rss),
        "runs": len(times_ms),
    }


def load_scenarios(path: Path = SCENARIOS) -> list[dict[str, Any]]:
    with path.open("rb") as handle:
        scenarios = tomllib.load(handle).get("scenario", [])
    names = [s["name"] for s in scenarios]
    if len(names) != len(set(names)):
        raise ValueError("duplicate scenario names")
    return scenarios


def run_once(command: list[str], env: dict[str, str]) -> tuple[float, int, int]:
    """Run once; return wall milliseconds, exit code and peak RSS in KB."""
    start = time.perf_counter()
    process = subprocess.Popen(  # noqa: S603 - command is built from the scenario file
        command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env
    )
    _, status, usage = os.wait4(process.pid, 0)
    elapsed = (time.perf_counter() - start) * 1000
    process.returncode = os.waitstatus_to_exitcode(status)
    return elapsed, process.returncode, rss_kb(usage.ru_maxrss)


def measure(
    binary: Path, scenario: dict[str, Any], runs: int, warmup: int
) -> dict[str, Any]:
    accepted = set(scenario.get("accept_exits", [0]))
    args = [str(a).replace("{root}", str(ROOT)) for a in scenario.get("args", [])]
    command = [str(binary), *args]
    times: list[float] = []
    rss: list[int] = []
    with tempfile.TemporaryDirectory(prefix="slopvac-bench-") as scratch:
        for index in range(warmup + runs):
            env = dict(os.environ)
            if scenario.get("cold"):
                cache = Path(scratch) / f"cache-{index}"
                env["XDG_CACHE_HOME"] = str(cache)
                env["SLOPVAC_CACHE_DIR"] = str(cache)
            elapsed, code, peak = run_once(command, env)
            if code not in accepted:
                return {"name": scenario["name"], "status": "error", "exit_code": code}
            if index >= warmup:
                times.append(elapsed)
                rss.append(peak)
    return {"name": scenario["name"], "status": "ok", **summarize(times, rss)}


def version_of(binary: Path) -> str:
    try:
        out = subprocess.run(
            [str(binary), "--version"],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    except OSError:
        return "unknown"
    return out.stdout.strip() or "unknown"


def check_budgets(result: dict[str, Any], budgets: dict[str, Any]) -> list[str]:
    """Return one line per exceeded budget; budgets are upper bounds."""
    over: list[str] = []
    size_limit = budgets.get("binary_size_bytes")
    if size_limit is not None and result["binary_size_bytes"] > size_limit:
        over.append(f"binary size {result['binary_size_bytes']} > {size_limit} bytes")
    for name, limits in budgets.get("scenario", {}).items():
        found = next((s for s in result["scenarios"] if s["name"] == name), None)
        if found is None or found["status"] != "ok":
            over.append(f"{name}: no measurement")
            continue
        for key in ("median_ms", "p95_ms", "peak_rss_kb"):
            if key in limits and found[key] > limits[key]:
                over.append(f"{name}: {key} {found[key]} > {limits[key]}")
    return over


def regressed(base: float, head: float, max_ratio: float, min_delta: float) -> bool:
    """Slower by more than the ratio AND by more than the absolute floor."""
    return head > base * max_ratio and head - base > min_delta


def compare_table(
    base: dict[str, Any], head: dict[str, Any], ratio: float, delta_ms: float
) -> tuple[str, bool]:
    lines = [
        "| Scenario | Base median ms | Head median ms | Change | Peak RSS KB |",
        "|---|---:|---:|---:|---:|",
    ]
    any_regressed = False
    base_by_name = {s["name"]: s for s in base["scenarios"]}
    for scenario in head["scenarios"]:
        before = base_by_name.get(scenario["name"])
        if scenario["status"] != "ok" or before is None or before["status"] != "ok":
            lines.append(f"| {scenario['name']} | n/a | n/a | not comparable | n/a |")
            continue
        bad = regressed(before["median_ms"], scenario["median_ms"], ratio, delta_ms)
        any_regressed |= bad
        change = (
            scenario["median_ms"] / before["median_ms"] - 1
            if before["median_ms"]
            else 0.0
        )
        flag = " REGRESSION" if bad else ""
        lines.append(
            f"| {scenario['name']} | {before['median_ms']} | {scenario['median_ms']} | {change:+.1%}{flag} | {scenario['peak_rss_kb']} |"
        )
    size_change = (
        head["binary_size_bytes"] / base["binary_size_bytes"] - 1
        if base["binary_size_bytes"]
        else 0.0
    )
    lines.append(
        f"\nBinary size: {base['binary_size_bytes']} -> {head['binary_size_bytes']} bytes ({size_change:+.1%})"
    )
    return "\n".join(lines), any_regressed


def cmd_run(args: argparse.Namespace) -> int:
    binary = Path(args.binary).resolve()
    if not binary.is_file():
        print(f"binary not found: {binary}", file=sys.stderr)
        return 2
    scenarios = load_scenarios(Path(args.scenarios))
    if args.scenario:
        scenarios = [s for s in scenarios if s["name"] == args.scenario]
        if not scenarios:
            print(f"unknown scenario: {args.scenario}", file=sys.stderr)
            return 2
    result = {
        "schema": SCHEMA,
        "binary": str(binary),
        "version": version_of(binary),
        "binary_size_bytes": binary.stat().st_size,
        "platform": f"{platform.system().lower()}-{platform.machine().lower()}",
        "cpu_count": os.cpu_count(),
        "scenarios": [measure(binary, s, args.runs, args.warmup) for s in scenarios],
    }
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        Path(args.output).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    for scenario in result["scenarios"]:
        if scenario["status"] != "ok":
            print(
                f"note: {scenario['name']} exited {scenario['exit_code']}; not measured",
                file=sys.stderr,
            )
    if args.budgets:
        with open(args.budgets, "rb") as handle:
            over = check_budgets(result, tomllib.load(handle))
        for line in over:
            print(f"over budget: {line}", file=sys.stderr)
        if over and args.enforce:
            return 1
    return 0


def cmd_compare(args: argparse.Namespace) -> int:
    base = json.loads(Path(args.base).read_text(encoding="utf-8"))
    head = json.loads(Path(args.head).read_text(encoding="utf-8"))
    table, bad = compare_table(base, head, args.max_ratio, args.min_delta_ms)
    print(table)
    return 1 if bad and args.fail_on_regression else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="measure a binary")
    run.add_argument("--binary", required=True)
    run.add_argument("--scenarios", default=str(SCENARIOS))
    run.add_argument("--scenario")
    run.add_argument("--runs", type=int, default=RUNS)
    run.add_argument("--warmup", type=int, default=WARMUP)
    run.add_argument("--output")
    run.add_argument("--budgets")
    run.add_argument("--enforce", action="store_true")
    run.set_defaults(func=cmd_run)
    compare = sub.add_parser("compare", help="compare two result files")
    compare.add_argument("base")
    compare.add_argument("head")
    compare.add_argument("--max-ratio", type=float, default=1.25)
    compare.add_argument("--min-delta-ms", type=float, default=5.0)
    compare.add_argument("--fail-on-regression", action="store_true")
    compare.set_defaults(func=cmd_compare)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
