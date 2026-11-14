#!/usr/bin/env python3
"""Helpers for the release install smoke test (.github/workflows/release-smoke.yml).

Subcommands:
  parse-tag TAG [--latest-tag LATEST]
      validate a release tag and print ``version=``, ``stable=``, ``dist=`` and
      ``latest=`` lines for ``$GITHUB_OUTPUT``; exit 1 for any tag that is not
      vX.Y.Z or vX.Y.Z-rc.N. ``dist`` is false before major 3, which release.yml
      does not build with dist. ``latest`` is true only for a stable tag equal to
      LATEST, the repository's latest release: the tap serves only that one.
  expect-version TAG OUTPUT
      exit 0 when OUTPUT is exactly what ``slopvac --version`` prints for TAG
  verify-sha256 FILE CHECKSUM_FILE
      exit 0 when FILE hashes to the digest in CHECKSUM_FILE

The tag arrives from a release event or a workflow input, so the workflow passes
it only through the environment and this script is the single place that decides
whether it is well formed.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

# No leading zeros in any numeric field (SemVer); kept in step with release_provenance.py.
_NUM = r"(?:0|[1-9][0-9]*)"
_TAG = re.compile(rf"v(?P<version>{_NUM}\.{_NUM}\.{_NUM}(?P<pre>-rc\.[1-9][0-9]*)?)")
# Same value as GATED_FROM_MAJOR in release_provenance.py (a test keeps them equal).
DIST_FROM_MAJOR = 3
_DIGEST = re.compile(r"[0-9a-fA-F]{64}")


@dataclass(frozen=True)
class ReleaseTag:
    version: str
    stable: bool
    dist: bool


def parse_tag(tag: str) -> ReleaseTag:
    match = _TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a release tag (expected vX.Y.Z or vX.Y.Z-rc.N): {tag!r}")
    major = int(match["version"].split(".")[0])
    return ReleaseTag(
        version=match["version"],
        stable=match["pre"] is None,
        dist=major >= DIST_FROM_MAJOR,
    )


def is_latest(tag: str, latest_tag: str) -> bool:
    """True when ``tag`` is a stable release and is the repository's latest release."""
    return parse_tag(tag).stable and tag == latest_tag


def expect_version(tag: str, output: str) -> None:
    """``slopvac --version`` prints ``slopvac <version>``; the tag names that version."""
    expected = f"slopvac {parse_tag(tag).version}"
    actual = output.strip()
    if actual != expected:
        raise ValueError(f"expected {expected!r}, got {actual!r}")


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_sha256(path: Path, checksum_file: Path) -> None:
    """The checksum file holds ``<digest>  <name>`` (or just the digest)."""
    words = checksum_file.read_text(encoding="utf-8").split()
    if not words or _DIGEST.fullmatch(words[0]) is None:
        raise ValueError(f"{checksum_file} holds no sha256 digest")
    actual = sha256_of(path)
    if actual != words[0].lower():
        raise ValueError(
            f"{path.name}: sha256 {actual} does not match {words[0].lower()}"
        )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    parse = sub.add_parser("parse-tag")
    parse.add_argument("tag")
    parse.add_argument("--latest-tag", default="")
    expect = sub.add_parser("expect-version")
    expect.add_argument("tag")
    expect.add_argument("output")
    verify = sub.add_parser("verify-sha256")
    verify.add_argument("file", type=Path)
    verify.add_argument("checksum_file", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "parse-tag":
            release = parse_tag(args.tag)
            latest = is_latest(args.tag, args.latest_tag)
            print(f"version={release.version}")
            print(f"stable={str(release.stable).lower()}")
            print(f"dist={str(release.dist).lower()}")
            print(f"latest={str(latest).lower()}")
        elif args.command == "expect-version":
            expect_version(args.tag, args.output)
        else:
            verify_sha256(args.file, args.checksum_file)
    except (ValueError, OSError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
