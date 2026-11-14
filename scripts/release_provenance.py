#!/usr/bin/env python3
"""Candidate tags and the provenance gate for stable releases.

A candidate is an ``rc/vX.Y.Z-rc.N`` tag cut on demand. A stable ``vX.Y.Z`` tag
may publish only when a passed candidate for the same version tested the exact
tree being promoted.

Subcommands:
  parse-tag TAG   print the Cargo and PyPI versions for a candidate tag
  check-stable TAG  exit 0 when the stable tag descends from a passed candidate
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Callable, Sequence
from dataclasses import dataclass

_RC_TAG = re.compile(r"rc/v(?P<base>\d+\.\d+\.\d+)-rc\.(?P<n>[1-9]\d*)")
_STABLE_TAG = re.compile(r"v(?P<base>\d+\.\d+\.\d+)")
CANDIDATE_WORKFLOW = "candidate.yml"

Runner = Callable[[Sequence[str]], str]


@dataclass(frozen=True)
class Candidate:
    cargo_version: str
    pypi_version: str


def parse_candidate_tag(tag: str) -> Candidate:
    match = _RC_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a candidate tag (expected rc/vX.Y.Z-rc.N): {tag}")
    base, n = match["base"], match["n"]
    return Candidate(cargo_version=f"{base}-rc.{n}", pypi_version=f"{base}rc{n}")


def run_command(argv: Sequence[str]) -> str:
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(argv)} failed: {done.stderr.strip()}")
    return done.stdout


def _run_proves(run: dict, candidate: str, sha: str) -> bool:
    """A run proves a candidate only when it is that tag's push run at that commit."""
    return (
        run.get("conclusion") == "success"
        and run.get("status") == "completed"
        and run.get("event") == "push"
        and run.get("head_sha") == sha
        and run.get("head_branch") == candidate
        and run.get("path") == f".github/workflows/{CANDIDATE_WORKFLOW}"
    )


def check_stable(tag: str, repo: str, runner: Runner = run_command) -> str:
    """Return the candidate tag that proves ``tag``, or raise ``ValueError``."""
    match = _STABLE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    base = match["base"]
    stable_tree = runner(["git", "rev-parse", f"{tag}^{{tree}}"]).strip()
    refs = runner(["git", "tag", "--list", f"rc/v{base}-rc.*"]).split()
    candidates = [t for t in refs if _RC_TAG.fullmatch(t)]
    if not candidates:
        raise ValueError(f"no candidate tag rc/v{base}-rc.N exists")
    # Newest candidate first, so the message names the closest miss.
    candidates.sort(key=lambda t: int(_RC_TAG.fullmatch(t)["n"]), reverse=True)
    problems: list[str] = []
    for candidate in candidates:
        tree = runner(["git", "rev-parse", f"{candidate}^{{tree}}"]).strip()
        if tree != stable_tree:
            problems.append(f"{candidate}: tree differs from {tag}")
            continue
        sha = runner(["git", "rev-parse", f"{candidate}^{{commit}}"]).strip()
        runs = json.loads(
            runner(
                [
                    "gh",
                    "api",
                    (
                        f"repos/{repo}/actions/workflows/{CANDIDATE_WORKFLOW}/runs"
                        f"?head_sha={sha}&event=push&status=completed&per_page=100"
                    ),
                ]
            )
        )
        if any(
            _run_proves(run, candidate, sha) for run in runs.get("workflow_runs", [])
        ):
            return candidate
        problems.append(
            f"{candidate}: no successful {CANDIDATE_WORKFLOW} run of that tag at {sha}"
        )
    raise ValueError(
        "no passed candidate matches the stable tag: " + "; ".join(problems)
    )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawTextHelpFormatter
    )
    sub = parser.add_subparsers(dest="command", required=True)
    parse = sub.add_parser("parse-tag")
    parse.add_argument("tag")
    check = sub.add_parser("check-stable")
    check.add_argument("tag")
    check.add_argument("--repo", required=True, help="owner/name")
    args = parser.parse_args(argv)
    try:
        if args.command == "parse-tag":
            candidate = parse_candidate_tag(args.tag)
            print(f"cargo={candidate.cargo_version}")
            print(f"pypi={candidate.pypi_version}")
        else:
            print(f"candidate={check_stable(args.tag, args.repo)}")
    except (ValueError, RuntimeError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
