#!/usr/bin/env python3
"""Run the slopvac 3.0.0 release-readiness checklist and print the evidence.

Exit 0 when every item passes, 1 when any item fails, 2 when a tool is missing
and so an item could not run. An exit of 2 never counts as a pass.

Automated items run the configured commands and query the bead tracker. Manual
approvals (security review, signed artifacts, Action and hook contracts, the
policies document) pass only when `release-evidence.toml` records them.
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
from datetime import date
from pathlib import Path
from urllib.parse import urlparse

PASS, FAIL, INCOMPLETE = "pass", "fail", "incomplete"
_CONFIG = Path(__file__).with_name("release-gate.toml")
_EVIDENCE = Path(__file__).with_name("release-evidence.toml")

Runner = Callable[[Sequence[str]], subprocess.CompletedProcess[str]]


@dataclass(frozen=True)
class Result:
    id: str
    title: str
    status: str
    evidence: str


REPO_ROOT = Path(__file__).resolve().parents[1]

_NEXTEST = ("cargo", "nextest", "run", "--workspace", "--no-tests=fail", "-E")
_PYTEST = ("python3", "-m", "pytest", "-q")

# Each gate item runs exactly one trusted command. A config entry passes only
# when its `run` equals the argv here, so editing release-gate.toml cannot swap
# in a no-op, a looser filter or another runner. Items whose check does not
# exist yet point at its future path and fail until it does.
TRUSTED_COMMANDS: dict[str, tuple[str, ...]] = {
    "golden": (*_NEXTEST, "test(/^golden::/)"),
    "accuracy": (*_NEXTEST, "test(/^accuracy::/)"),
    "compat-preserve": (*_NEXTEST, "test(/^compat::preserve/)"),
    "conformance": (*_NEXTEST, "test(/^conformance::/)"),
    "conformance-contract": (*_PYTEST, "tests/contract"),
    "migration-e2e": (*_NEXTEST, "test(/^migration_e2e::/)"),
    "perf-budgets": (*_NEXTEST, "test(/^perf_budget::/)"),
    "perf-installed-binary": (
        "python3",
        "scripts/perf/binary_bench.py",
        "run",
        "--binary",
        "target/release/slopvac",
        "--budgets",
        "scripts/perf/binary-budgets.toml",
        "--enforce",
    ),
}
MANDATORY_SUITES = tuple(TRUSTED_COMMANDS)

# Installed artifacts must be signed and verified on each of these targets.
TARGET_TRIPLES = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
)

# Approvals a person gives. Each needs a passing record in the evidence file.
MANDATORY_EVIDENCE: dict[str, str] = {
    "security-review": "Security review sign-off (plugin host, trust store, OCI client, extraction)",
    **{
        f"signed-artifacts-{triple}": f"Signed installed artifacts on {triple}"
        for triple in TARGET_TRIPLES
    },
    "action-contract": "GitHub Action contract",
    "hook-contract": "Published pre-commit hook contract",
    "policies-doc": "Policies document (docs/contract/policies.md) approved",
    "accessibility-manual-pass": "Manual keyboard and screen-reader pass (policies section 2.3)",
}
_EVIDENCE_KEYS = {"id", "status", "evidence", "approver", "date"}
_ISSUE_KEYS = {"id": str, "status": str, "labels": list}


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    """Run a gate command from the repository root, whatever the caller's directory."""
    return subprocess.run(
        argv, capture_output=True, text=True, check=False, cwd=REPO_ROOT
    )


def _tracker_json(runner: Runner, *args: str) -> list[dict]:
    """Return the tracker's issues.

    Raises ValueError or TypeError unless stdout is a JSON array of objects that carry the
    keys the checks rely on, so empty or malformed output is never "no issues".
    """
    done = runner(["bd", *args, "--json", "--limit", "0"])
    label = f"bd {' '.join(args)}"
    if done.returncode != 0:
        raise RuntimeError(done.stderr.strip() or f"{label} failed")
    try:
        issues = json.loads(done.stdout)
    except json.JSONDecodeError as err:
        raise ValueError(f"{label}: output is not JSON ({err.msg})") from err
    if not isinstance(issues, list):
        raise TypeError(f"{label}: expected a JSON array")
    for index, issue in enumerate(issues):
        if not isinstance(issue, dict):
            raise TypeError(f"{label}: record {index} is not an object")
        for key, kind in _ISSUE_KEYS.items():
            value = issue.get(key)
            if not isinstance(value, kind) or (kind is str and not value):
                raise ValueError(f"{label}: record {index} lacks a valid `{key}`")
    return issues


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


def _pin_problem(suite: str, argv: object) -> str | None:
    """Why `argv` is not the trusted command for `suite`, or None."""
    trusted = TRUSTED_COMMANDS[suite]
    if not isinstance(argv, list) or not all(isinstance(a, str) for a in argv):
        return "run must be a list of strings"
    if tuple(argv) != trusted:
        return f"run must be exactly `{' '.join(trusted)}`"
    return None


def check_suites_configured(config: dict) -> Result:
    """Fail unless every mandatory suite is configured with its pinned runner."""
    title = "Every mandatory suite is configured"
    entries: dict[object, dict] = {}
    problems: list[str] = []
    for entry in config.get("command", []):
        if not isinstance(entry, dict):
            problems.append("a command entry is not a table")
        elif not isinstance(entry.get("id"), str):
            problems.append(f"command id must be a string, got {entry.get('id')!r}")
        elif entry.get("id") in entries:
            problems.append(f"duplicate: {entry.get('id')}")
        else:
            entries[entry.get("id")] = entry
    missing = [suite for suite in MANDATORY_SUITES if suite not in entries]
    if missing:
        problems.append(f"missing: {', '.join(missing)}")
    extra = [str(i) for i in entries if i not in TRUSTED_COMMANDS]
    if extra:
        problems.append(f"not a mandatory suite: {', '.join(extra)}")
    for suite in MANDATORY_SUITES:
        if suite in entries:
            bad = _pin_problem(suite, entries[suite].get("run"))
            if bad:
                problems.append(f"{suite}: {bad}")
    if problems:
        return Result("suites-configured", title, FAIL, "; ".join(problems))
    return Result("suites-configured", title, PASS, ", ".join(MANDATORY_SUITES))


def check_command(runner: Runner, entry: dict) -> Result:
    argv = entry["run"]
    tool = "cargo-nextest" if argv[:2] == ["cargo", "nextest"] else argv[0]
    if shutil.which(argv[0]) is None or shutil.which(tool) is None:
        missing = argv[0] if shutil.which(argv[0]) is None else tool
        return Result(
            entry["id"], entry["title"], INCOMPLETE, f"{missing} is not installed"
        )
    done = runner(argv)
    command = f"`{' '.join(argv)}` exit {done.returncode}"
    if done.returncode == 0:
        return Result(entry["id"], entry["title"], PASS, command)
    tail = (done.stdout + done.stderr).strip().splitlines()[-3:]
    return Result(entry["id"], entry["title"], FAIL, f"{command}: {' / '.join(tail)}")


def _evidence_problem(item: dict) -> str | None:
    """Why a recorded item does not count as approved, or None."""
    if item.get("status") != "pass":
        return f"status is {item.get('status')!r}, not 'pass'"
    links = item.get("evidence")
    if not isinstance(links, list) or not links:
        return "no evidence links"
    for link in links:
        parsed = urlparse(link) if isinstance(link, str) else None
        if parsed is None or parsed.scheme != "https" or not parsed.netloc:
            return f"evidence link is not https: {link!r}"
    approver = item.get("approver")
    if not isinstance(approver, str) or not approver.strip():
        return "no approver"
    if not isinstance(item.get("date"), date):
        return "date is not a TOML date (YYYY-MM-DD)"
    return None


def check_evidence(path: Path) -> list[Result]:
    """One result per mandatory approval, read from the evidence file.

    A missing or malformed file fails every item. Unknown keys, unknown or
    duplicate items add an `evidence-file` failure.
    """
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as err:
        reason = f"evidence file {path.name} unusable: {err}"
        return [
            Result(f"evidence:{key}", title, FAIL, reason)
            for key, title in MANDATORY_EVIDENCE.items()
        ]
    problems: list[str] = []
    unknown_top = sorted(set(data) - {"item"})
    if unknown_top:
        problems.append(f"unknown top-level keys: {', '.join(unknown_top)}")
    items = data.get("item", [])
    if not isinstance(items, list) or not all(isinstance(i, dict) for i in items):
        items = []
        problems.append("`item` must be an array of tables")
    records: dict[object, dict] = {}
    for item in items:
        name = item.get("id")
        if not isinstance(name, str):
            problems.append(f"item id must be a string, got {name!r}")
            continue
        unknown = sorted(set(item) - _EVIDENCE_KEYS)
        if unknown:
            problems.append(f"item {name!r}: unknown keys {', '.join(unknown)}")
        if name in records:
            problems.append(f"duplicate item {name!r}")
            continue
        if name not in MANDATORY_EVIDENCE:
            problems.append(f"unknown item {name!r}")
        records[name] = item
    results = []
    for key, title in MANDATORY_EVIDENCE.items():
        if key not in records:
            results.append(Result(f"evidence:{key}", title, FAIL, "no record"))
            continue
        item = records[key]
        bad = _evidence_problem(item)
        if bad:
            results.append(Result(f"evidence:{key}", title, FAIL, bad))
        else:
            detail = f"{item['approver']} {item['date']}: {', '.join(item['evidence'])}"
            results.append(Result(f"evidence:{key}", title, PASS, detail))
    if problems:
        results.append(
            Result("evidence-file", "Evidence file is valid", FAIL, "; ".join(problems))
        )
    return results


def _guard(result_id: str, title: str, fn: Callable[[], Result]) -> Result:
    try:
        return fn()
    except FileNotFoundError as err:
        return Result(result_id, title, INCOMPLETE, f"{err.filename} is not installed")
    except (RuntimeError, TypeError, ValueError) as err:
        return Result(result_id, title, INCOMPLETE, str(err))


def run_gate(
    config: dict, runner: Runner = run_process, evidence: Path = _EVIDENCE
) -> list[Result]:
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
    # Only pinned commands run; check_suites_configured reports the rest.
    results += [
        check_command(runner, entry)
        for entry in config.get("command", [])
        if isinstance(entry, dict)
        and isinstance(entry.get("id"), str)
        and entry.get("id") in TRUSTED_COMMANDS
        and _pin_problem(entry["id"], entry.get("run")) is None
    ]
    return results + check_evidence(evidence)


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
    parser.add_argument("--evidence", type=Path, default=_EVIDENCE)
    parser.add_argument(
        "--json", action="store_true", help="print JSON instead of a table"
    )
    args = parser.parse_args(argv)
    config = tomllib.loads(args.config.read_text(encoding="utf-8"))
    results = run_gate(config, run_process, args.evidence)
    if args.json:
        print(json.dumps([asdict(r) for r in results], indent=2))
    else:
        print(render_markdown(results))
    return exit_code(results)


if __name__ == "__main__":
    sys.exit(main())
