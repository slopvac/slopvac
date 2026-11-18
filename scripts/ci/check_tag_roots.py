#!/usr/bin/env python3
"""Fail when a tag points at a commit outside slopvac's history.

Slopvac has one root commit. A tag whose commit has any other root, or more than
one root, comes from an unrelated history. Two modes share the same check:

* default: check every tag in the repository in the current directory;
* --pre-push: the same check, run before a push with no network access. It ignores
  which refs the push carries, because prek does not forward them: a foreign tag in
  the clone is refused whatever is being pushed.

A tag that is a symbolic ref is foreign and is never followed. Offending tags are
listed as plain full ref names, one per line, with control characters replaced.
The script never changes the repository; CONTRIBUTING.md explains how to remove
a foreign tag by hand.

Exit 0 when every checked tag passes, 1 when a tag fails, 2 when git fails.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from collections.abc import Iterable

# The one place the root is recorded. SLOPVAC_ROOT_COMMIT overrides it for tests.
ROOT_COMMIT = "b516adb5ed8789b2b737c5876f75f2fbc809e755"
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


def offending(tags: Iterable[tuple[str, str, bool]], root: str) -> list[str]:
    """Full refnames of tags whose commit does not have `root` as its only root."""
    cache: dict[str, list[str] | None] = {}
    bad = []
    for ref, sha, symbolic in tags:
        if symbolic:
            bad.append(ref)
            continue
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


def parse_records(output: bytes) -> list[tuple[str, str, bool]]:
    """Parse `refname NUL objectname NUL symref NUL LF` records.

    Only NUL separates fields. Ref names may hold Unicode spaces and line
    separators, so no Unicode-aware split is safe. Names decode with
    surrogateescape, which keeps them exact.
    """
    fields = output.split(b"\0")
    if fields.pop() not in (b"", b"\n") or len(fields) % 3:
        raise GitError("unexpected git for-each-ref output")
    tags = []
    for i in range(0, len(fields), 3):
        ref, sha, symref = fields[i : i + 3]
        ref = ref.removeprefix(b"\n").decode("utf-8", "surrogateescape")
        if not ref.startswith(TAG_PREFIX) or not sha:
            raise GitError("unexpected git for-each-ref record")
        tags.append((ref, sha.decode("ascii", "replace"), bool(symref)))
    return tags


def all_tags() -> list[tuple[str, str, bool]]:
    """Every tag ref as (full refname, object id, is a symbolic ref)."""
    require_full_history()
    result = subprocess.run(
        [
            "git",
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)%00",
            TAG_PREFIX,
        ],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", "replace").strip()
        raise GitError(message or "git for-each-ref failed")
    return parse_records(result.stdout)


def printable(text: str) -> str:
    """Escape every non-printable character, so a name cannot drive a terminal.

    Applied only where names are printed; the names themselves stay exact.
    """
    return "".join(c if c.isprintable() else escape(c) for c in text)


def escape(char: str) -> str:
    code = ord(char)
    return f"\\u{code:04x}" if code <= 0xFFFF else f"\\U{code:08x}"


def report(bad: list[str], checked: int, root: str, local: bool) -> None:
    where = "local tags" if local else "tags"
    print(
        f"error: {len(bad)} of {checked} {where} are not in slopvac's history "
        f"(expected the only root commit to be {root}).",
        file=sys.stderr,
    )
    if local:
        print(
            "Push refused. These local tags are outside slopvac's history, whether or "
            "not the push carries them; CONTRIBUTING.md explains how to remove them "
            "by hand.",
            file=sys.stderr,
        )
    for ref in bad[:LIST_LIMIT]:
        print(printable(ref), file=sys.stderr)
    if len(bad) > LIST_LIMIT:
        print(f"... and {len(bad) - LIST_LIMIT} more", file=sys.stderr)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--pre-push",
        action="store_true",
        help="pre-push mode: same check, with a message for local tags",
    )
    parser.add_argument(
        "--root",
        default=os.environ.get("SLOPVAC_ROOT_COMMIT", ROOT_COMMIT),
        help="expected root commit (default: SLOPVAC_ROOT_COMMIT or slopvac's root)",
    )
    args = parser.parse_args(argv)
    try:
        tags = all_tags()
        bad = offending(tags, args.root)
    except GitError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    if bad:
        report(bad, len(tags), args.root, args.pre_push)
        return 1
    print(f"checked {len(tags)} tags: 0 offending")
    return 0


if __name__ == "__main__":
    sys.exit(main())
