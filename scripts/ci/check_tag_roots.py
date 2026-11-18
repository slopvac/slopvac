#!/usr/bin/env python3
"""Fail when a tag points at a commit outside slopvac's history.

Slopvac has one root commit. A tag whose commit has any other root, or more than
one root, comes from an unrelated history. Two modes share the same check:

* default: check every tag in the repository in the current directory;
* --pre-push: check only the pushed tags, with no network access. It reads git's
  pre-push lines from stdin. prek consumes those lines and passes one ref in
  PRE_COMMIT_REMOTE_BRANCH and PRE_COMMIT_TO_REF instead, so the check falls back
  to that ref when stdin carries none.

Exit 0 when every checked tag passes, 1 when a tag fails, 2 when git fails.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections.abc import Iterable, Mapping

# The one place the root is recorded. SLOPVAC_ROOT_COMMIT overrides it for tests.
ROOT_COMMIT = "b516adb5ed8789b2b737c5876f75f2fbc809e755"
ZERO_SHA = "0" * 40
TAG_PREFIX = "refs/tags/"
LIST_LIMIT = 50


class GitError(RuntimeError):
    """A git command failed for a reason other than a bad tag."""


def git(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=False)


def roots_of(commit: str) -> list[str] | None:
    """Root commits reachable from `commit`, or None when it is not a commit."""
    peeled = git("rev-parse", "--verify", "--quiet", f"{commit}^{{commit}}")
    if peeled.returncode != 0:
        return None
    result = git("rev-list", "--max-parents=0", peeled.stdout.strip())
    if result.returncode != 0:
        raise GitError(result.stderr.strip() or "git rev-list failed")
    return result.stdout.split()


def offending(pairs: Iterable[tuple[str, str]], root: str) -> list[str]:
    """Names of tags whose commit does not have `root` as its only root."""
    return [name for name, commit in pairs if roots_of(commit) != [root]]


def all_tags() -> list[tuple[str, str]]:
    result = git("for-each-ref", "--format=%(refname) %(objectname)", TAG_PREFIX)
    if result.returncode != 0:
        raise GitError(result.stderr.strip() or "git for-each-ref failed")
    return [
        (ref.removeprefix(TAG_PREFIX), sha)
        for ref, sha in (line.split() for line in result.stdout.splitlines())
    ]


def pushed_tags(lines: Iterable[str]) -> list[tuple[str, str]]:
    """Tag refs from pre-push lines; deletions and other refs are skipped."""
    pairs = []
    for line in lines:
        fields = line.split()
        if len(fields) != 4:
            continue
        _local_ref, local_sha, remote_ref, _remote_sha = fields
        if local_sha == ZERO_SHA or not remote_ref.startswith(TAG_PREFIX):
            continue
        pairs.append((remote_ref.removeprefix(TAG_PREFIX), local_sha))
    return pairs


def prek_tag(environ: Mapping[str, str]) -> list[tuple[str, str]]:
    """The one pushed ref prek exposes through its environment, if it is a tag."""
    ref = environ.get("PRE_COMMIT_REMOTE_BRANCH", "")
    sha = environ.get("PRE_COMMIT_TO_REF", "")
    if not ref.startswith(TAG_PREFIX) or not sha or sha == ZERO_SHA:
        return []
    return [(ref.removeprefix(TAG_PREFIX), sha)]


def report(bad: list[str], checked: int, root: str) -> None:
    print(
        f"error: {len(bad)} of {checked} tags are not in slopvac's history "
        f"(expected the only root commit to be {root}).",
        file=sys.stderr,
    )
    for name in bad[:LIST_LIMIT]:
        print(f"  {name}", file=sys.stderr)
    if len(bad) > LIST_LIMIT:
        print(f"  ... and {len(bad) - LIST_LIMIT} more", file=sys.stderr)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--pre-push",
        action="store_true",
        help="read pre-push lines from stdin instead of scanning every tag",
    )
    parser.add_argument(
        "--root",
        default=os.environ.get("SLOPVAC_ROOT_COMMIT", ROOT_COMMIT),
        help="expected root commit (default: SLOPVAC_ROOT_COMMIT or slopvac's root)",
    )
    args = parser.parse_args(argv)
    try:
        if args.pre_push:
            lines = [] if sys.stdin.isatty() else sys.stdin.read().splitlines()
            pairs = pushed_tags(lines) or prek_tag(os.environ)
        else:
            pairs = all_tags()
        bad = offending(pairs, args.root)
    except GitError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    if bad:
        report(bad, len(pairs), args.root)
        return 1
    print(f"checked {len(pairs)} tags: 0 offending")
    return 0


if __name__ == "__main__":
    sys.exit(main())
