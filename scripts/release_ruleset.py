#!/usr/bin/env python3
"""Fail closed unless an active ruleset makes the release tag immutable.

A tag-writer who can move a ``vX.Y.Z`` tag after the provenance check could
publish a different commit under it. Only a repository ruleset that blocks
updates, deletions and non-fast-forward pushes on ``refs/tags/v*`` closes that
window, and only a maintainer can create it. This script reads the rulesets
(read-only) and refuses to continue when the protection is missing, inactive,
or cannot be read.

Usage: release_ruleset.py check TAG --repo OWNER/NAME
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Callable, Sequence

REQUIRED_RULES = frozenset({"update", "deletion", "non_fast_forward"})
PER_PAGE = 100
MAX_PAGES = 5
_NUM = r"(?:0|[1-9][0-9]*)"  # SemVer: no leading zeros
_TAG = re.compile(rf"v{_NUM}\.{_NUM}\.{_NUM}")

Runner = Callable[[Sequence[str]], str]

HINT = (
    "Create a repository ruleset that targets tags matching refs/tags/v*, is "
    "active, and has the update, deletion and non_fast_forward rules (see "
    "docs/release/distribution.md). If the ruleset exists, the workflow token "
    "may be unable to read it."
)


def run_command(argv: Sequence[str]) -> str:
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(argv)} failed: {done.stderr.strip()}")
    return done.stdout


def _get(path: str, runner: Runner) -> object:
    return json.loads(
        runner(["gh", "api", "-H", "X-GitHub-Api-Version: 2022-11-28", path])
    )


def _list_tag_rulesets(repo: str, runner: Runner) -> list[dict]:
    found: list[dict] = []
    for page in range(1, MAX_PAGES + 1):
        batch = _get(
            f"repos/{repo}/rulesets?targets=tag&per_page={PER_PAGE}&page={page}",
            runner,
        )
        if not isinstance(batch, list):
            raise TypeError("the rulesets response is not a list")
        found.extend(batch)
        if len(batch) < PER_PAGE:
            return found
    raise ValueError(f"more than {MAX_PAGES * PER_PAGE} rulesets; refusing to guess")


# GitHub matches ruleset ref patterns with fnmatch under File::FNM_PATHNAME and adds
# the tokens ~ALL and ~DEFAULT_BRANCH (docs.github.com, "Creating rulesets for a
# repository", "Targeting by name"). Python's fnmatch lets `*` cross `/` and has no
# `**/` zero-directory rule, so it must not decide here. Instead only include
# patterns known to cover every refs/tags/v<semver> ref count, and any exclude that
# might match counts as excluding. Both choices fail closed.
_TAG_INCLUDES = frozenset(
    {"~ALL", "refs/tags/*", "refs/tags/**/*", "refs/tags/**", "refs/tags/v*"}
)
_WILDCARDS = "*?[\\"


def _includes(pattern: str, ref: str) -> bool:
    return pattern == ref or pattern in _TAG_INCLUDES


def _might_exclude(pattern: str, ref: str) -> bool:
    """False only when ``pattern`` provably cannot match ``ref``."""
    if pattern == "~ALL":
        return True
    if pattern == "~DEFAULT_BRANCH":
        return False  # a branch token; it never names a tag
    cut = min((pattern.find(c) for c in _WILDCARDS if c in pattern), default=-1)
    if cut == -1:
        return pattern == ref
    # A wildcard only widens the match after its literal prefix.
    return ref.startswith(pattern[:cut])


def _covers(ruleset: dict, ref: str) -> bool:
    """The ruleset's ref conditions surely include ``ref`` and no exclude may match it."""
    ref_name = (ruleset.get("conditions") or {}).get("ref_name") or {}
    included = any(_includes(p, ref) for p in ref_name.get("include", []))
    excluded = any(_might_exclude(p, ref) for p in ref_name.get("exclude", []))
    return included and not excluded


# The repository admin role. Admins can change any ruleset anyway, so a bypass for
# them adds no power; any other bypass actor can move or delete a protected tag.
ADMIN_ROLE_ID = 5
NO_BYPASS_FIELD = (
    "the ruleset response has no bypass_actors field, so a bypass for a write role "
    "cannot be ruled out. GitHub returns that field only to a token that can "
    "administer the repository: the release app's installation token needs the "
    "Administration: read permission"
)


def _admin_only_bypass(ruleset: dict) -> bool:
    """True when every bypass actor is the repository admin role or an organization admin."""
    actors = ruleset.get("bypass_actors")
    if not isinstance(actors, list):
        raise TypeError(f"ruleset {ruleset.get('id')}: {NO_BYPASS_FIELD}")
    return all(
        actor.get("actor_type") == "OrganizationAdmin"
        or (
            actor.get("actor_type") == "RepositoryRole"
            and actor.get("actor_id") == ADMIN_ROLE_ID
        )
        for actor in actors
    )


def check_tag_protection(tag: str, repo: str, runner: Runner | None = None) -> None:
    """Raise ``ValueError`` unless active tag rulesets cover ``tag`` with every required rule."""
    if _TAG.fullmatch(tag) is None:
        raise ValueError(f"not a release tag (expected vX.Y.Z): {tag}")
    runner = runner or run_command
    ref = f"refs/tags/{tag}"
    enforced: set[str] = set()
    for summary in _list_tag_rulesets(repo, runner):
        if summary.get("target") != "tag" or summary.get("enforcement") != "active":
            continue
        ruleset = _get(f"repos/{repo}/rulesets/{summary['id']}", runner)
        if not isinstance(ruleset, dict):
            raise TypeError("a ruleset response is not an object")
        if ruleset.get("target") != "tag" or ruleset.get("enforcement") != "active":
            continue
        if _covers(ruleset, ref) and _admin_only_bypass(ruleset):
            enforced |= {rule.get("type") for rule in ruleset.get("rules", [])}
    missing = sorted(REQUIRED_RULES - enforced)
    if missing:
        raise ValueError(
            f"no active tag ruleset covers {ref} with the rules: {', '.join(missing)}, and no bypass for a non-admin actor"
        )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check")
    check.add_argument("tag")
    check.add_argument("--repo", required=True, help="owner/name")
    args = parser.parse_args(argv)
    try:
        check_tag_protection(args.tag, args.repo)
    except (ValueError, TypeError, RuntimeError, KeyError) as err:
        print(f"error: {err}\n{HINT}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
