#!/usr/bin/env python3
"""Candidate tags and the provenance gate for stable releases.

A candidate is an ``rc/vX.Y.Z-rc.N`` tag cut on demand. A stable ``vX.Y.Z`` tag
may publish only when a passed candidate for the same version tested the exact
tree being promoted.

Subcommands:
  parse-tag TAG
      print the Cargo and PyPI versions for a candidate tag
  check-stable TAG --repo OWNER/NAME --sha SHA
      exit 0 when TAG points at SHA (the commit the workflow runs on) and a
      passed candidate tested that tree
  verify-tag TAG SHA
      exit 0 when TAG on the remote still resolves to SHA; run it right before
      publishing, so a tag moved after the check cannot be released
  requires-gate VERSION
      print true when a stable release of VERSION needs the gate
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
_STABLE_VERSION = re.compile(r"(?P<major>\d+)\.\d+\.\d+")
_PRE_VERSION = re.compile(r"\d+\.\d+\.\d+(?:a|b|rc)\d+")
CANDIDATE_WORKFLOW = "candidate.yml"
# Releases from this major version on need a passed candidate. 2.x candidates are
# untagged branch builds, so they cannot satisfy a candidate-tag gate.
GATED_FROM_MAJOR = 3
PER_PAGE = 100
MAX_PAGES = 10

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


def requires_gate(version: str) -> bool:
    """True for a stable X.Y.Z version of major 3 or later.

    A prerelease (``3.0.0rc1``) is a candidate itself and is never gated.
    """
    match = _STABLE_VERSION.fullmatch(version)
    if match is not None:
        return int(match["major"]) >= GATED_FROM_MAJOR
    if _PRE_VERSION.fullmatch(version):
        return False
    raise ValueError(f"not a version: {version}")


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


def _candidate_runs(repo: str, sha: str, runner: Runner) -> list[dict]:
    """Every completed push run of the candidate workflow at ``sha``, page by page."""
    runs: list[dict] = []
    for page in range(1, MAX_PAGES + 1):
        data = json.loads(
            runner(
                [
                    "gh",
                    "api",
                    (
                        f"repos/{repo}/actions/workflows/{CANDIDATE_WORKFLOW}/runs"
                        f"?head_sha={sha}&event=push&status=completed"
                        f"&per_page={PER_PAGE}&page={page}"
                    ),
                ]
            )
        )
        batch = data.get("workflow_runs", [])
        runs.extend(batch)
        if len(batch) < PER_PAGE:
            return runs
    raise ValueError(
        f"more than {MAX_PAGES * PER_PAGE} candidate runs at {sha}; refusing to guess"
    )


def check_stable(
    tag: str, repo: str, event_sha: str, runner: Runner = run_command
) -> str:
    """Return the candidate tag that proves ``tag``, or raise ``ValueError``.

    ``event_sha`` is the commit the release workflow runs on. The tag must still
    point at it, so a tag moved after the event cannot borrow this check.
    """
    match = _STABLE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    base = match["base"]
    stable_commit = runner(["git", "rev-parse", f"{tag}^{{commit}}"]).strip()
    if stable_commit != event_sha:
        raise ValueError(
            f"{tag} points at {stable_commit}, not the workflow commit {event_sha}"
        )
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
        if any(
            _run_proves(run, candidate, sha)
            for run in _candidate_runs(repo, sha, runner)
        ):
            return candidate
        problems.append(
            f"{candidate}: no successful {CANDIDATE_WORKFLOW} run of that tag at {sha}"
        )
    raise ValueError(
        "no passed candidate matches the stable tag: " + "; ".join(problems)
    )


def remote_tag_commit(tag: str, runner: Runner = run_command) -> str:
    """The commit ``tag`` resolves to on the remote, peeled for annotated tags."""
    ref = f"refs/tags/{tag}"
    lines = runner(["git", "ls-remote", "--tags", "origin", ref, f"{ref}^{{}}"])
    found: dict[str, str] = {}
    for line in lines.splitlines():
        sha, _, name = line.partition("\t")
        found[name.strip()] = sha.strip()
    resolved = found.get(f"{ref}^{{}}") or found.get(ref)
    if not resolved:
        raise ValueError(f"{tag} does not exist on the remote")
    return resolved


def verify_tag(tag: str, sha: str, runner: Runner = run_command) -> None:
    """Fail unless the remote tag still points at ``sha``."""
    if _STABLE_TAG.fullmatch(tag) is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    current = remote_tag_commit(tag, runner)
    if current != sha:
        raise ValueError(f"{tag} moved to {current}; the release was built from {sha}")


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
    check.add_argument("--sha", required=True, help="commit the workflow runs on")
    verify = sub.add_parser("verify-tag")
    verify.add_argument("tag")
    verify.add_argument("sha")
    gate = sub.add_parser("requires-gate")
    gate.add_argument("version")
    args = parser.parse_args(argv)
    try:
        if args.command == "parse-tag":
            candidate = parse_candidate_tag(args.tag)
            print(f"cargo={candidate.cargo_version}")
            print(f"pypi={candidate.pypi_version}")
        elif args.command == "check-stable":
            print(f"candidate={check_stable(args.tag, args.repo, args.sha)}")
        elif args.command == "verify-tag":
            verify_tag(args.tag, args.sha)
        else:
            print("true" if requires_gate(args.version) else "false")
    except (ValueError, RuntimeError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
