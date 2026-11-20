"""Docs pin this repository's Action to a full commit SHA, never to a tag.

The release flow creates only immutable ``vX.Y.Z`` tags and no moving major
tag, so a ``@v3`` reference would not resolve. See docs/release/distribution.md.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]

# The marker stands in for the SHA of the v3.0.0 tag until that tag exists.
PLACEHOLDER = "<full commit SHA of the v3.0.0 tag>"

_USES = re.compile(
    r"uses:\s*slopvac/slopvac(?:/[^@\s]*)?@(?P<ref><[^>]+>|[^\s#]+)(?P<rest>[^\n]*)"
)
# The 2.x surface (README.md until the 3.0 cutover) still names its release tag.
_LEGACY = re.compile(r"v[12]\.\d+\.\d+")
_SHA = re.compile(r"[0-9a-f]{40}")
_VERSION_COMMENT = re.compile(r"\s+# v\d+\.\d+\.\d+\s*$")

_EXCLUDED_PARTS = {"golden", "node_modules", ".venv", "target"}


def problems(text: str) -> list[str]:
    """Return one message per Action reference in ``text`` that breaks the pin rule."""
    found = []
    for match in _USES.finditer(text):
        ref, rest = match["ref"], match["rest"]
        if _LEGACY.fullmatch(ref):
            continue
        if ref == PLACEHOLDER or _SHA.fullmatch(ref):
            if not _VERSION_COMMENT.match(rest):
                found.append(f"{ref}: missing '# vX.Y.Z' comment")
        else:
            found.append(f"{ref}: not a full commit SHA")
    return found


def doc_files() -> list[Path]:
    paths = [*ROOT.glob("README*.md"), *ROOT.glob("docs/**/*.md"), *ROOT.glob("action-v3/**/*.md")]
    return sorted(
        p
        for p in paths
        if not p.name.startswith("CHANGELOG") and not _EXCLUDED_PARTS & set(p.relative_to(ROOT).parts)
    )


SHA = "0123456789abcdef0123456789abcdef01234567"


@pytest.mark.parametrize(
    "line",
    [
        "- uses: slopvac/slopvac@v3",
        "- uses: slopvac/slopvac@v3.0",
        "- uses: slopvac/slopvac@v3.0.0",
        "- uses: slopvac/slopvac@main",
        "- uses: slopvac/slopvac@abc1234",
        f"- uses: slopvac/slopvac@{SHA}",
        f"- uses: slopvac/slopvac@{SHA} # latest",
        "- uses: slopvac/slopvac/action-v3@v3",
        f"- uses: slopvac/slopvac@{SHA.upper()} # v3.0.0",
    ],
)
def test_rejects_unpinned_or_uncommented_reference(line: str) -> None:
    assert problems(line)


@pytest.mark.parametrize(
    "line",
    [
        f"- uses: slopvac/slopvac@{SHA} # v3.0.0",
        f"      uses: slopvac/slopvac/action-v3@{SHA} # v3.1.12",
        f"- uses: slopvac/slopvac@{PLACEHOLDER} # v3.0.0",
        "- uses: actions/checkout@v7",
        "- uses: slopvac/slopvac@v2.18.1 # x-release-please-version",
    ],
)
def test_accepts_pinned_reference(line: str) -> None:
    assert problems(line) == []


def test_repository_docs_pin_the_action() -> None:
    files = doc_files()
    assert files, "no documentation files found"
    bad = {
        str(path.relative_to(ROOT)): found
        for path in files
        if (found := problems(path.read_text(encoding="utf-8")))
    }
    assert bad == {}
