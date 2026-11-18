#!/usr/bin/env python3
"""Fail closed unless a reviewed ruleset makes the release tag immutable.

A tag-writer who can move a ``vX.Y.Z`` tag after the provenance check could
publish a different commit under it. Only a repository ruleset that blocks
updates, deletions and non-fast-forward pushes on ``refs/tags/v*`` closes that
window, and only a maintainer can create it.

GitHub shows ``bypass_actors`` only to callers that can write the ruleset, so the
read-only release token cannot inspect them. Instead ``check`` trusts a ruleset
only when its ID and ``updated_at`` match a committed pin (``updated_at`` changes
on every edit) and GitHub reports ``current_user_can_bypass`` as ``never`` for the
reading token. ``update-pins`` is the only writer of the pin file and needs an
admin token, which sees ``bypass_actors``.

Usage:
    release_ruleset.py check TAG --repo OWNER/NAME
    release_ruleset.py update-pins --repo OWNER/NAME
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Callable, Sequence
from datetime import datetime
from pathlib import Path

REQUIRED_RULES = frozenset({"update", "deletion", "non_fast_forward"})
PER_PAGE = 100
MAX_PAGES = 5
_NUM = r"(?:0|[1-9][0-9]*)"  # SemVer: no leading zeros
_TAG = re.compile(rf"v{_NUM}\.{_NUM}\.{_NUM}")

PINS_PATH = Path(__file__).with_name("release_ruleset_pins.json")
PINS_COMMENT = (
    "Tag rulesets that protect release tags, pinned by ID and updated_at. "
    "Change this file only through release_ruleset.py update-pins, and review it "
    "like a change to the ruleset itself."
)
SAMPLE_TAG = "v3.0.0"

Runner = Callable[[Sequence[str]], str]

HINT = (
    "Create a repository ruleset that targets tags matching refs/tags/v*, is "
    "active, and has the update, deletion and non_fast_forward rules (see "
    "docs/release/distribution.md). If you changed or added a ruleset, review it, "
    "then have a repository admin run `scripts/release_ruleset.py update-pins "
    "--repo OWNER/NAME` and commit the pin file."
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


def _admin_only_bypass(actors: object) -> bool:
    """True when every bypass actor is the repository admin role or an organization admin."""
    if not isinstance(actors, list):
        return False
    return all(
        actor.get("actor_type") == "OrganizationAdmin"
        or (
            actor.get("actor_type") == "RepositoryRole"
            and actor.get("actor_id") == ADMIN_ROLE_ID
        )
        for actor in actors
    )


def _instant(value: object) -> datetime | None:
    """Parse a GitHub timestamp so equal instants compare equal across UTC offsets."""
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError:
        return None
    return parsed if parsed.tzinfo is not None else None


def load_pins(repo: str, path: Path = PINS_PATH) -> dict[int, datetime]:
    """Return the pinned ``updated_at`` instant by ruleset ID for ``repo``."""
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("repo") != repo:
        raise ValueError(f"{path.name} pins rulesets of {data.get('repo')}, not {repo}")
    pins: dict[int, datetime] = {}
    for key, entry in data.get("rulesets", {}).items():
        instant = _instant(entry.get("updated_at"))
        if instant is None:
            raise ValueError(f"{path.name}: ruleset {key} has no valid updated_at")
        pins[int(key)] = instant
    return pins


def _rejection(ruleset: dict, pins: dict[int, datetime]) -> str | None:
    """Why a covering ruleset cannot be trusted, or ``None`` when it can."""
    rid = ruleset.get("id")
    pinned = pins.get(rid) if isinstance(rid, int) else None
    if pinned is None:
        return f"ruleset {rid} is not pinned"
    if _instant(ruleset.get("updated_at")) != pinned:
        return f"ruleset {rid} changed since it was pinned (updated_at {ruleset.get('updated_at')})"
    if ruleset.get("current_user_can_bypass") != "never":
        return f"ruleset {rid} lets the reading token bypass it (current_user_can_bypass={ruleset.get('current_user_can_bypass')})"
    if "bypass_actors" in ruleset and not _admin_only_bypass(ruleset["bypass_actors"]):
        return f"ruleset {rid} has a bypass for a non-admin actor"
    return None


def check_tag_protection(
    tag: str,
    repo: str,
    runner: Runner | None = None,
    pins: dict[int, datetime] | None = None,
) -> None:
    """Raise ``ValueError`` unless pinned, active tag rulesets cover ``tag`` with every required rule."""
    if _TAG.fullmatch(tag) is None:
        raise ValueError(f"not a release tag (expected vX.Y.Z): {tag}")
    runner = runner or run_command
    pins = load_pins(repo) if pins is None else pins
    ref = f"refs/tags/{tag}"
    enforced: set[str] = set()
    rejected: list[str] = []
    for summary in _list_tag_rulesets(repo, runner):
        ruleset = _get(f"repos/{repo}/rulesets/{summary['id']}", runner)
        if not isinstance(ruleset, dict):
            raise TypeError("a ruleset response is not an object")
        if ruleset.get("target") != "tag" or ruleset.get("enforcement") != "active":
            continue
        if not _covers(ruleset, ref):
            continue
        reason = _rejection(ruleset, pins)
        if reason is None:
            enforced |= {rule.get("type") for rule in ruleset.get("rules", [])}
        else:
            rejected.append(reason)
    missing = sorted(REQUIRED_RULES - enforced)
    if missing:
        detail = f" ({'; '.join(rejected)})" if rejected else ""
        raise ValueError(
            f"no trusted active tag ruleset covers {ref} with the rules: {', '.join(missing)}{detail}"
        )


def pin_entries(repo: str, runner: Runner | None = None) -> dict[str, dict[str, str]]:
    """Read the rulesets to pin; raise unless the caller is an admin who sees them all.

    Pins every active tag ruleset that covers a sample release tag and has a
    required rule, and refuses unless together they enforce all required rules.
    """
    runner = runner or run_command
    ref = f"refs/tags/{SAMPLE_TAG}"
    entries: dict[str, dict[str, str]] = {}
    enforced: set[str] = set()
    for summary in _list_tag_rulesets(repo, runner):
        if summary.get("target") != "tag" or summary.get("enforcement") != "active":
            continue
        ruleset = _get(f"repos/{repo}/rulesets/{summary['id']}", runner)
        if not isinstance(ruleset, dict):
            raise TypeError("a ruleset response is not an object")
        types = {rule.get("type") for rule in ruleset.get("rules", [])}
        if not _covers(ruleset, ref) or not types & REQUIRED_RULES:
            continue
        if "bypass_actors" not in ruleset:
            raise ValueError(
                f"ruleset {ruleset.get('id')} shows no bypass_actors: run update-pins with a repository admin token"
            )
        if not _admin_only_bypass(ruleset["bypass_actors"]):
            raise ValueError(
                f"ruleset {ruleset.get('id')} has a bypass for a non-admin actor; fix the ruleset before pinning"
            )
        if _instant(ruleset.get("updated_at")) is None:
            raise ValueError(f"ruleset {ruleset.get('id')} has no valid updated_at")
        entries[str(ruleset["id"])] = {
            "name": str(ruleset.get("name", "")),
            "updated_at": ruleset["updated_at"],
        }
        enforced |= types
    missing = sorted(REQUIRED_RULES - enforced)
    if missing:
        raise ValueError(
            f"no active tag ruleset covers {ref} with the rules: {', '.join(missing)}"
        )
    return dict(sorted(entries.items(), key=lambda item: int(item[0])))


def update_pins(
    repo: str, runner: Runner | None = None, path: Path = PINS_PATH
) -> None:
    """Rewrite the pin file from the live rulesets; the only writer of that file."""
    entries = pin_entries(repo, runner)
    document = {"_comment": PINS_COMMENT, "repo": repo, "rulesets": entries}
    path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check")
    check.add_argument("tag")
    check.add_argument("--repo", required=True, help="owner/name")
    pins = sub.add_parser(
        "update-pins", help="rewrite the pin file; needs an admin token"
    )
    pins.add_argument("--repo", required=True, help="owner/name")
    args = parser.parse_args(argv)
    try:
        if args.command == "check":
            check_tag_protection(args.tag, args.repo)
        else:
            update_pins(args.repo)
    except (ValueError, TypeError, RuntimeError, KeyError, OSError) as err:
        print(f"error: {err}\n{HINT}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
