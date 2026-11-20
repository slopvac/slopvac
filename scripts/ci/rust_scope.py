#!/usr/bin/env python3
"""Decide which Rust crates a pull request must check, and fail closed.

cargo-delta maps the changed files to workspace crates. This script runs it and turns the
answer into cargo arguments for the CI jobs. Any doubt widens the scope to the whole
workspace: a pull request never tests less than cargo-delta proved safe.

Inputs, from the environment:
  EVENT_NAME   the GitHub event; only `pull_request` is ever scoped
  FORCE_ALL    `true` when the routing was edited or the event is the merge queue or main
  BASE_REF     the pull request's base branch; its `origin/` ref must be fetched
  GITHUB_OUTPUT  file that receives the outputs

Outputs:
  rust_scope     `all` or `scoped`
  rust_modified  cargo arguments for fmt and clippy: `--workspace` or `-p NAME ...`
  rust_affected  cargo arguments for build and test: `--workspace` or `-p NAME ...`
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

ALL = "--workspace"
# Cargo package names. Anything else is refused before it reaches a shell word split.
NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]*\Z")
TIMEOUT_SECONDS = 600


@dataclass(frozen=True)
class Scope:
    """The crates to check, or the whole workspace with the reason why."""

    modified: tuple[str, ...] | None
    affected: tuple[str, ...] | None
    reason: str

    @property
    def scoped(self) -> bool:
        return self.affected is not None


def everything(reason: str) -> Scope:
    return Scope(None, None, reason)


def parse_impact(text: str) -> Scope:
    """Read cargo-delta's JSON, or fall back to the whole workspace."""
    if not text.strip():
        return everything("cargo-delta reported no impact at all")
    try:
        impact = json.loads(text)
        modified = impact["Modified"]
        affected = impact["Affected"]
    except (ValueError, KeyError, TypeError) as error:
        return everything(f"cargo-delta output was not understood ({error!r})")
    for tier in (modified, affected):
        if not isinstance(tier, list) or not all(
            isinstance(n, str) and NAME.match(n) for n in tier
        ):
            return everything("cargo-delta named a package that is not a plain name")
    if not modified or not affected:
        return everything("cargo-delta found changes but mapped them to no crate")
    if not set(modified) <= set(affected):
        return everything("cargo-delta's modified crates are not all affected")
    return Scope(tuple(sorted(modified)), tuple(sorted(affected)), "scoped")


def run_delta(delta: Sequence[str], config: str, base_ref: str) -> Scope:
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "impact.json"
        command = [
            *delta,
            "-c",
            config,
            "impact",
            "--base-ref",
            f"origin/{base_ref}",
            "--output",
            str(out),
            "-f",
            "json",
        ]
        try:
            done = subprocess.run(
                command, check=False, timeout=TIMEOUT_SECONDS, stdout=sys.stderr
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            return everything(f"cargo-delta did not run ({error!r})")
        if done.returncode != 0:
            return everything(f"cargo-delta exited {done.returncode}")
        return parse_impact(out.read_text() if out.exists() else "")


def decide(env: dict[str, str], delta: Sequence[str], config: str) -> Scope:
    if env.get("EVENT_NAME") != "pull_request":
        return everything(
            f"{env.get('EVENT_NAME', 'unknown')} runs the whole workspace"
        )
    if env.get("FORCE_ALL") != "false":
        return everything("routing edits and the queue run the whole workspace")
    base_ref = env.get("BASE_REF", "")
    if not re.fullmatch(r"[A-Za-z0-9._/-]+", base_ref):
        return everything("the base branch name is missing or unusual")
    return run_delta(delta, config, base_ref)


def render(scope: Scope) -> dict[str, str]:
    def args(names: tuple[str, ...] | None) -> str:
        return ALL if names is None else " ".join(f"-p {n}" for n in names)

    return {
        "rust_scope": "scoped" if scope.scoped else "all",
        "rust_modified": args(scope.modified),
        "rust_affected": args(scope.affected),
    }


def main(argv: Sequence[str]) -> int:
    delta = ["cargo", "delta"]
    config = ".github/cargo-delta.toml"
    args = list(argv)
    while args:
        flag = args.pop(0)
        if flag == "--delta":
            delta = args.pop(0).split()
        elif flag == "--config":
            config = args.pop(0)
        else:
            print(f"unknown argument {flag}", file=sys.stderr)
            return 2
    scope = decide(dict(os.environ), delta, config)
    outputs = render(scope)
    print(f"Rust scope: {outputs['rust_scope']} ({scope.reason})")
    for key, value in outputs.items():
        print(f"  {key}={value}")
    if scope.scoped:
        print(
            "::notice::Rust checks cover the changed crates and their dependents only"
        )
    else:
        print(f"::notice::Rust checks cover the whole workspace: {scope.reason}")
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            for key, value in outputs.items():
                handle.write(f"{key}={value}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
