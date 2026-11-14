#!/usr/bin/env python3
"""Generate the exclude-pattern parity table for slopvac-engine discovery.

The 2.x reference matched `exclude` with `pathspec` (gitwildmatch) through
`PathSpec.match_file`. The Rust `Excludes` type must agree with it, so each row
here is a pattern list, a relative path, and what pathspec answers. The Rust test
`exclude_matches_the_pathspec_reference` reads the generated file.

    python scripts/discover_exclude_parity.py          # rewrite the fixture
    python scripts/discover_exclude_parity.py --check  # fail if it is stale

Row format: patterns joined by TAB-free "\\x1f", TAB, path, TAB, 1 if excluded.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pathspec

FIXTURE = (
    Path(__file__).resolve().parent.parent
    / "crates/slopvac-engine/tests/fixtures/exclude_parity.tsv"
)

DEFAULTS = [
    "**/node_modules/**",
    "**/apm_modules/**",
    "**/.venv/**",
    "**/dist/**",
    "**/build/**",
    "**/CHANGELOG.md",
    "**/.git/**",
]

CASES: list[tuple[list[str], list[str]]] = [
    # Defaults shipped in 2.x Config.exclude.
    (
        DEFAULTS,
        [
            "CHANGELOG.md",
            "docs/CHANGELOG.md",
            "docs/CHANGELOG.md.bak",
            "node_modules/a/readme.md",
            "web/node_modules/a/readme.md",
            "build/out.md",
            "src/build/out.md",
            "src/rebuild/out.md",
            ".git/config",
            "docs/guide.md",
        ],
    ),
    # Precedence: the last matching pattern wins, across file and ancestor matches.
    (["*.md", "!keep/"], ["keep/a.md", "other/a.md", "a.md"]),
    (["!keep/", "*.md"], ["keep/a.md", "other/a.md"]),
    (["docs/", "!docs/keep.md"], ["docs/keep.md", "docs/other.md"]),
    (["*.md", "!README.md"], ["README.md", "sub/README.md", "a.md"]),
    (["!README.md", "*.md"], ["README.md"]),
    (["*.md", "!a.md", "a.md"], ["a.md"]),
    # Directory patterns and trailing slash.
    (["drafts/"], ["drafts/a.md", "x/drafts/a.md", "drafts", "mydrafts/a.md"]),
    (["drafts"], ["drafts/a.md", "x/drafts/a.md", "drafts", "mydrafts/a.md"]),
    (["drafts/*"], ["drafts/a.md", "drafts/sub/b.md", "x/drafts/a.md"]),
    (["drafts/**"], ["drafts/a.md", "drafts/sub/b.md", "drafts", "x/drafts/a.md"]),
    # Anchoring.
    (["/docs"], ["docs/a.md", "x/docs/a.md", "docs"]),
    (["/docs/**"], ["docs/a.md", "docs/docs/a.md", "x/docs/a.md"]),
    (["docs/a.md"], ["docs/a.md", "x/docs/a.md"]),
    (["a.md"], ["a.md", "x/a.md", "x/ba.md"]),
    # Double star.
    (["**/foo"], ["foo", "a/foo", "a/b/foo/c.md", "foobar"]),
    (["a/**/b.md"], ["a/b.md", "a/x/b.md", "a/x/y/b.md", "b.md"]),
    (["**/a/**"], ["a/b.md", "x/a/b.md", "x/ab/b.md"]),
    # Wildcards and classes.
    (["*.m?"], ["a.md", "a.mx", "a.mdx"]),
    (["[ab].md"], ["a.md", "b.md", "c.md", "x/a.md"]),
    (["[!ab].md"], ["a.md", "c.md"]),
    (["[a-c].md"], ["b.md", "d.md"]),
    (["docs/*.md"], ["docs/a.md", "docs/sub/a.md"]),
    # Escapes, comments and blanks.
    (["\\#note.md"], ["#note.md", "note.md"]),
    (["#comment", "a.md"], ["#comment", "a.md"]),
    (["\\!bang.md"], ["!bang.md", "bang.md"]),
    (["a\\*b.md"], ["a*b.md", "axb.md"]),
    (["", "   ", "a.md"], ["a.md", "b.md"]),
    # Braces are literal in gitwildmatch, not alternation.
    (["a{b,c}.md"], ["a{b,c}.md", "ab.md", "ac.md"]),
    (["\\{x\\}.md", "{y}.md"], ["{x}.md", "{y}.md", "x.md"]),
    # Case sensitivity.
    (["README.md"], ["README.md", "readme.md"]),
]


def rows() -> list[str]:
    out = []
    for patterns, paths in CASES:
        spec = pathspec.PathSpec.from_lines("gitwildmatch", patterns)
        for path in paths:
            out.append(
                f"{chr(0x1F).join(patterns)}\t{path}\t{int(spec.match_file(path))}"
            )
    return out


def main() -> int:
    text = "\n".join(rows()) + "\n"
    if "--check" in sys.argv:
        if not FIXTURE.exists() or FIXTURE.read_text(encoding="utf-8") != text:
            print(f"{FIXTURE} is stale; run scripts/discover_exclude_parity.py")
            return 1
        return 0
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
