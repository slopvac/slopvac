#!/usr/bin/env python3
"""Delete old versions of the OCI client CI package on GHCR.

The `oci-live` workflow pushes scratch artifacts to ghcr.io/slopvac/oci-client-ci and
this script, run weekly by the same workflow, removes the versions older than a cutoff
(14 days by default). It never touches a version that carries a tag with the given keep
prefix, which is how the running workflow names what it has just pushed.

Environment: GH_TOKEN, the workflow's GITHUB_TOKEN, read by `gh`. The job needs
`packages: write`; GitHub documents that this lets a workflow delete versions of a
package its repository published (see docs/engineering.md, approved departures).

Exit codes: 0 done, 1 a delete failed, 2 the run was invalid or incomplete.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

ORG = "slopvac"
# Cleanup is for this package only. Any other name is refused before a request is made.
PACKAGE = "oci-client-ci"
DEFAULT_DAYS = 14
KEEP_PREFIX = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]*\Z")
GH_TIMEOUT_SECONDS = 120


@dataclass(frozen=True)
class Version:
    """A package version: its id, creation time and tags."""

    id: int
    created_at: datetime
    tags: tuple[str, ...]


class Invalid(Exception):
    """The input is not what the GitHub API documents, so the run is incomplete."""


def parse_version(raw: object) -> Version:
    """Read one version object. Anything that lacks an id, a time or tag metadata is refused."""
    if not isinstance(raw, dict):
        raise Invalid("a version entry is not an object")
    version_id = raw.get("id")
    created = raw.get("created_at")
    metadata = raw.get("metadata")
    container = metadata.get("container") if isinstance(metadata, dict) else None
    tags = container.get("tags") if isinstance(container, dict) else None
    if not isinstance(version_id, int) or isinstance(version_id, bool):
        raise Invalid("a version entry has no integer id")
    if not isinstance(created, str):
        raise Invalid(f"version {version_id} has no created_at")
    # Absent or broken tag metadata is not "no tags": a protected tag may hide behind it.
    if not isinstance(tags, list) or not all(isinstance(t, str) for t in tags):
        raise Invalid(f"version {version_id} has missing or malformed tag metadata")
    try:
        when = datetime.fromisoformat(created)
    except ValueError as err:
        raise Invalid(f"version {version_id} has an unreadable created_at") from err
    if when.tzinfo is None:
        raise Invalid(f"version {version_id} has a created_at without a zone")
    return Version(version_id, when, tuple(tags))


def select_stale(
    versions: Iterable[Version], now: datetime, days: int, keep_prefix: str
) -> list[Version]:
    """The versions older than `days` that carry no tag starting with `keep_prefix`."""
    cutoff = now - timedelta(days=days)
    return [
        v
        for v in versions
        if v.created_at < cutoff and not any(t.startswith(keep_prefix) for t in v.tags)
    ]


def gh(args: Sequence[str]) -> str:
    """Run `gh api` and return stdout. A failure raises with a fixed message and no output."""
    try:
        done = subprocess.run(  # noqa: S603
            ["gh", "api", *args],  # noqa: S607
            capture_output=True,
            text=True,
            timeout=GH_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as err:
        raise Invalid(f"gh api did not run: {type(err).__name__}") from err
    if done.returncode != 0:
        # gh's own text can carry request details, so only the code is reported.
        raise Invalid(f"gh api failed with exit code {done.returncode}")
    return done.stdout


def pages(text: str) -> list[object]:
    """The JSON values `gh api --paginate` printed, one per page, in order."""
    decoder = json.JSONDecoder()
    values: list[object] = []
    index = 0
    while True:
        while index < len(text) and text[index].isspace():
            index += 1
        if index >= len(text):
            return values
        try:
            value, index = decoder.raw_decode(text, index)
        except json.JSONDecodeError as err:
            raise Invalid("gh api returned text that is not JSON") from err
        values.append(value)


def list_versions(base: str) -> list[Version]:
    """All active versions of the package, over every page."""
    out = gh(["--paginate", f"{base}/versions?per_page=100"])
    versions: list[Version] = []
    for page in pages(out):
        if not isinstance(page, list):
            raise Invalid("a page of versions is not a list")
        versions.extend(parse_version(entry) for entry in page)
    return versions


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--package", default=PACKAGE)
    parser.add_argument("--days", type=int, default=DEFAULT_DAYS)
    parser.add_argument(
        "--keep-prefix",
        required=True,
        help="never delete a version with a tag that starts with this, e.g. ci-<run id>-",
    )
    parser.add_argument("--dry-run", action="store_true")
    # Only the tests pass this, so their fixtures never depend on the wall clock.
    parser.add_argument("--now", type=datetime.fromisoformat, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    if args.package != PACKAGE:
        print(f"error: only the {PACKAGE} package may be cleaned", file=sys.stderr)
        return 2
    if args.now is not None and args.now.tzinfo is None:
        print("error: --now needs a time zone", file=sys.stderr)
        return 2
    if args.days < 1:
        print("error: --days must be at least 1", file=sys.stderr)
        return 2
    if not KEEP_PREFIX.match(args.keep_prefix):
        print("error: --keep-prefix is not a valid tag prefix", file=sys.stderr)
        return 2

    base = f"/orgs/{ORG}/packages/container/{args.package}"
    try:
        versions = list_versions(base)
    except Invalid as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    stale = select_stale(versions, args.now or datetime.now(UTC), args.days, args.keep_prefix)
    print(f"{len(versions)} versions, {len(stale)} older than {args.days} days")

    failed = 0
    for version in stale:
        if args.dry_run:
            print(f"would delete version {version.id}")
            continue
        try:
            gh(["-X", "DELETE", f"{base}/versions/{version.id}"])
        except Invalid as err:
            print(f"error: version {version.id}: {err}", file=sys.stderr)
            failed += 1
            continue
        print(f"deleted version {version.id}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
