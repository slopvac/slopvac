#!/usr/bin/env python3
"""Fail when a tag points at a commit outside slopvac's history.

Slopvac has one root commit. A tag whose commit has any other root, or more than
one root, comes from an unrelated history. Two modes share the same check:

* default: check every tag in the repository in the current directory;
* --pre-push: the same check, run before a push with no network access, with a
  message that names --delete-foreign-local-tags to remove the offenders locally. It ignores which
  refs the push carries, because prek does not forward them: a foreign tag in the
  clone is refused whatever is being pushed;
* --delete-foreign-local-tags: delete the offending tags from this clone by full
  refname, with no shell involved.

Exit 0 when every checked tag passes, 1 when a tag fails, 2 when git fails.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path

# The one place the root is recorded. SLOPVAC_ROOT_COMMIT overrides it for tests.
ROOT_COMMIT = "b516adb5ed8789b2b737c5876f75f2fbc809e755"
TAG_PREFIX = "refs/tags/"
DELETE_FLAG = "--delete-foreign-local-tags"
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
    """Full refnames of tags whose commit does not have `root` as its only root."""
    cache: dict[str, list[str] | None] = {}
    bad = []
    for ref, sha in pairs:
        if sha not in cache:
            cache[sha] = roots_of(sha)
        if cache[sha] != [root]:
            bad.append(ref)
    return bad


def require_full_history() -> None:
    """A shallow clone has artificial roots, so every tag would look foreign."""
    result = git("rev-parse", "--is-shallow-repository")
    if result.returncode != 0:
        raise GitError(result.stderr.strip() or "git rev-parse failed")
    if result.stdout.strip() == "true":
        raise GitError(
            "this repository is a shallow clone, so its roots are not real; "
            "run `git fetch --unshallow` first"
        )


def all_tags() -> list[tuple[str, str]]:
    require_full_history()
    result = git("for-each-ref", "--format=%(refname) %(objectname)", TAG_PREFIX)
    if result.returncode != 0:
        raise GitError(result.stderr.strip() or "git for-each-ref failed")
    return [
        (ref, sha) for ref, sha in (line.split() for line in result.stdout.splitlines())
    ]


def delete_refs(refs: list[str]) -> None:
    """Delete each ref by its full name. No shell and no name splitting is involved."""
    for ref in refs:
        if not ref.startswith(TAG_PREFIX):
            raise GitError(f"refusing to delete a ref outside {TAG_PREFIX}: {ref}")
        result = git("update-ref", "-d", ref)
        if result.returncode != 0:
            raise GitError(result.stderr.strip() or f"could not delete {ref}")


def report(bad: list[str], checked: int, root: str, hint: bool) -> None:
    where = "local tags" if hint else "tags"
    print(
        f"error: {len(bad)} of {checked} {where} are not in slopvac's history "
        f"(expected the only root commit to be {root}).",
        file=sys.stderr,
    )
    for ref in bad[:LIST_LIMIT]:
        print(f"  {ref.removeprefix(TAG_PREFIX)}", file=sys.stderr)
    if len(bad) > LIST_LIMIT:
        print(f"  ... and {len(bad) - LIST_LIMIT} more", file=sys.stderr)
    if hint:
        print(
            "Push refused: these tags exist in this clone, whether or not the push "
            "carries them. Delete exactly these local tags with:\n"
            f"  python3 {Path(__file__).as_posix()} {DELETE_FLAG}",
            file=sys.stderr,
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--pre-push",
        action="store_true",
        help="pre-push mode: same check, with a hint for deleting local tags",
    )
    parser.add_argument(
        "--root",
        default=os.environ.get("SLOPVAC_ROOT_COMMIT", ROOT_COMMIT),
        help="expected root commit (default: SLOPVAC_ROOT_COMMIT or slopvac's root)",
    )
    parser.add_argument(
        DELETE_FLAG,
        action="store_true",
        help="delete the foreign tags from this clone, by full refname, then exit",
    )
    args = parser.parse_args(argv)
    try:
        pairs = all_tags()
        bad = offending(pairs, args.root)
        if args.delete_foreign_local_tags:
            delete_refs(bad)
            print(f"deleted {len(bad)} local tags")
            return 0
    except GitError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    if bad:
        report(bad, len(pairs), args.root, args.pre_push)
        return 1
    print(f"checked {len(pairs)} tags: 0 offending")
    return 0


if __name__ == "__main__":
    sys.exit(main())
