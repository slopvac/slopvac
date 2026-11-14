#!/usr/bin/env python3
"""Run the slopvac 3.0.0 release-readiness checklist and print the evidence.

Exit 0 when every item passes, 1 when any item fails, 2 when a tool is missing
and so an item could not run. An exit of 2 never counts as a pass.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Callable, Sequence
from dataclasses import asdict, dataclass
from pathlib import Path

PASS, FAIL, INCOMPLETE = "pass", "fail", "incomplete"
_CONFIG = Path(__file__).with_name("release-gate.toml")

Runner = Callable[[Sequence[str]], subprocess.CompletedProcess[str]]


@dataclass(frozen=True)
class Result:
    id: str
    title: str
    status: str
    evidence: str


REPO_ROOT = Path(__file__).resolve().parents[1]

# Suites the 3.0.0 release cannot ship without. A config that omits one is an
# error, so deleting an entry from release-gate.toml cannot turn the gate green.
MANDATORY_SUITES = (
    "golden",
    "accuracy",
    "compat-preserve",
    "conformance",
    "migration-e2e",
    "perf-budgets",
)


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    """Run a gate command from the repository root, whatever the caller's directory."""
    return subprocess.run(
        argv, capture_output=True, text=True, check=False, cwd=REPO_ROOT
    )


def _tracker_json(runner: Runner, *args: str) -> list[dict]:
    done = runner(["bd", *args, "--json", "--limit", "0"])
    if done.returncode != 0:
        raise RuntimeError(done.stderr.strip() or f"bd {' '.join(args)} failed")
    return json.loads(done.stdout or "[]")


def _ids(issues: Sequence[dict]) -> list[str]:
    return sorted(issue["id"] for issue in issues)


def check_release_beads_closed(runner: Runner, label: str, allowed: set[str]) -> Result:
    title = f"All {label} beads closed"
    open_ids = [
        i
        for i in _ids(_tracker_json(runner, "list", "--label", label))
        if i not in allowed
    ]
    if open_ids:
        return Result(
            "release-beads", title, FAIL, f"{len(open_ids)} open: {', '.join(open_ids)}"
        )
    return Result("release-beads", title, PASS, "none open")


def check_no_open_stubs(runner: Runner) -> Result:
    title = "Zero open stub beads"
    open_ids = _ids(_tracker_json(runner, "list", "--label", "stub"))
    if open_ids:
        return Result("no-stubs", title, FAIL, f"open: {', '.join(open_ids)}")
    return Result("no-stubs", title, PASS, "none open")


def _mentions(text: str, bead_id: str) -> bool:
    """Match a whole bead id, so `x.1` does not match inside `x.12`."""
    return re.search(rf"(?<![\w.-]){re.escape(bead_id)}(?![\w]|\.\d)", text) is not None


def check_soft_blocked(runner: Runner) -> Result:
    """Fail when an open `soft-blocked` bead still has an open swap bead."""
    title = "No open soft-blocked bead with an open swap bead"
    swaps = _tracker_json(runner, "list", "--label", "stub")
    pairs: set[str] = set()
    for bead in _tracker_json(runner, "list", "--label", "soft-blocked"):
        text = json.dumps(bead)
        linked = {
            d.get("id") or d.get("depends_on_id")
            for d in bead.get("dependencies") or []
        }
        for swap in swaps:
            if (
                swap["id"] in linked
                or _mentions(text, swap["id"])
                or _mentions(json.dumps(swap), bead["id"])
            ):
                pairs.add(f"{bead['id']} -> {swap['id']}")
    if pairs:
        return Result("soft-blocked", title, FAIL, "; ".join(sorted(pairs)))
    return Result("soft-blocked", title, PASS, "none")


def check_suites_configured(config: dict) -> Result:
    """Fail unless every mandatory suite is configured and cannot pass with zero tests."""
    title = "Every mandatory suite is configured"
    entries = {entry.get("id"): entry for entry in config.get("command", [])}
    missing = [suite for suite in MANDATORY_SUITES if suite not in entries]
    vacuous = [
        suite
        for suite in MANDATORY_SUITES
        if suite in entries
        and entries[suite]["run"][:2] == ["cargo", "nextest"]
        and "--no-tests=fail" not in entries[suite]["run"]
    ]
    problems = [f"missing: {', '.join(missing)}"] if missing else []
    if vacuous:
        problems.append(f"can pass with zero tests: {', '.join(vacuous)}")
    if problems:
        return Result("suites-configured", title, FAIL, "; ".join(problems))
    return Result("suites-configured", title, PASS, ", ".join(MANDATORY_SUITES))


def check_command(runner: Runner, entry: dict) -> Result:
    argv = entry["run"]
    if shutil.which(argv[0]) is None:
        return Result(
            entry["id"], entry["title"], INCOMPLETE, f"{argv[0]} is not installed"
        )
    done = runner(argv)
    command = f"`{' '.join(argv)}` exit {done.returncode}"
    if done.returncode == 0:
        return Result(entry["id"], entry["title"], PASS, command)
    tail = (done.stdout + done.stderr).strip().splitlines()[-3:]
    return Result(entry["id"], entry["title"], FAIL, f"{command}: {' / '.join(tail)}")


def _guard(result_id: str, title: str, fn: Callable[[], Result]) -> Result:
    try:
        return fn()
    except FileNotFoundError as err:
        return Result(result_id, title, INCOMPLETE, f"{err.filename} is not installed")
    except (RuntimeError, ValueError) as err:
        return Result(result_id, title, INCOMPLETE, str(err))


def run_gate(config: dict, runner: Runner = run_process) -> list[Result]:
    beads = config.get("beads", {})
    label = beads.get("release_label", "release:3.0.0")
    allowed = set(beads.get("allowed_open", []))
    results = [
        _guard(
            "release-beads",
            f"All {label} beads closed",
            lambda: check_release_beads_closed(runner, label, allowed),
        ),
        _guard("no-stubs", "Zero open stub beads", lambda: check_no_open_stubs(runner)),
        _guard(
            "soft-blocked",
            "No open soft-blocked bead with an open swap bead",
            lambda: check_soft_blocked(runner),
        ),
        check_suites_configured(config),
    ]
    results += [check_command(runner, entry) for entry in config.get("command", [])]
    return results


def exit_code(results: Sequence[Result]) -> int:
    if any(r.status == FAIL for r in results):
        return 1
    return 2 if any(r.status == INCOMPLETE for r in results) else 0


def render_markdown(results: Sequence[Result]) -> str:
    lines = ["| Item | Status | Evidence |", "| --- | --- | --- |"]
    lines += [
        f"| {r.title} | {r.status.upper()} | {r.evidence.replace('|', '/')} |"
        for r in results
    ]
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=_CONFIG)
    parser.add_argument(
        "--json", action="store_true", help="print JSON instead of a table"
    )
    args = parser.parse_args(argv)
    config = tomllib.loads(args.config.read_text(encoding="utf-8"))
    results = run_gate(config)
    if args.json:
        print(json.dumps([asdict(r) for r in results], indent=2))
    else:
        print(render_markdown(results))
    return exit_code(results)


if __name__ == "__main__":
    sys.exit(main())
