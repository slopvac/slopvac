"""Which links a generated page really holds, checked against a CommonMark parser."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "docs"))

import build_site
import mdlinks
import starlight

# The shape of the reported attack: a license span that opens a link, and a
# homepage span that closes it two table rows later.
CROSS_ROW = (
    "| License | `` ](../ `` |\n"
    "| Homepage | ``x/../../protocol.md)<script>alert(1)</script>`` |\n"
)

CASES = {
    "plain link": ("see [a](x.md) now", ["x.md"]),
    "fragment": ("[a](x.md#top)", ["x.md#top"]),
    "json target": ("[s](v3/a.schema.json)", ["v3/a.schema.json"]),
    "two links": ("[a](x.md) and [b](y.md)", ["x.md", "y.md"]),
    "code span": ("`[a](x.md)`", []),
    "long code span": ("``a`b [a](x.md)``", []),
    "link after a code span": ("`c` [a](x.md)", ["x.md"]),
    "backtick fence": ("```text\n[a](x.md)\n```\n", []),
    "tilde fence": ("~~~\n[a](x.md)\n~~~\n", []),
    "longer closing fence": ("```\n[a](x.md)\n`````\n[b](y.md)\n", ["y.md"]),
    "short closing fence does not close": ("````\n```\n[a](x.md)\n", []),
    "unclosed fence": ("```\n[a](x.md)\n", []),
    "link after a fence": ("```\nx\n```\n\n[a](x.md)\n", ["x.md"]),
    "backtick in a backtick fence info": ("``` a`b\n[a](x.md)\n", ["x.md"]),
    "escaped opening bracket": ("\\[a](x.md)", []),
    "escaped closing bracket": ("[a\\](x.md)", []),
    "escaped backtick": ("\\`[a](x.md)\\`", ["x.md"]),
    "bracket pair needs an opener": ("a](x.md)", []),
    "newline in the destination": ("[a](x\n.md)", []),
    "space in the destination": ("[a](x .md)", []),
    "absolute link": ("[a](https://e.test/x.md)", []),
    "code span over lines": ("a `b\n[c](x.md)` d", []),
    "unmatched backtick": ("a ` [c](x.md)", ["x.md"]),
    "cross-row attack": (CROSS_ROW, []),
    "code span never spans table rows": (
        "| h |\n| - |\n| `a |\n| b` [c](x.md) |\n",
        ["x.md"],
    ),
}


def _hrefs(text: str) -> list[str]:
    parser = MarkdownIt("commonmark").enable("table")
    found = []
    for block in parser.parse(text):
        for token in block.children or []:
            href = token.attrGet("href") if token.type == "link_open" else None
            if (
                href
                and ":" not in str(href)
                and str(href).split("#")[0].endswith((".md", ".json"))
            ):
                found.append(str(href))
    return found


@pytest.mark.parametrize("name", CASES)
def test_links_are_the_ones_a_renderer_would_make(name: str) -> None:
    text, expected = CASES[name]
    found = [link.target + link.fragment for link in mdlinks.find_links(text)]
    assert found == expected
    assert _hrefs(text) == expected


def test_link_offsets_cover_the_destination_and_nothing_else() -> None:
    text = "go [a](x.md#h) on"
    (link,) = mdlinks.find_links(text)
    assert text[link.start : link.end] == "](x.md#h)"


def test_the_link_check_ignores_link_text_inside_code() -> None:
    pages = {"a/index.md": "`[x](missing.md)` and\n```\n[y](gone.md)\n```\n"}
    assert build_site.broken_links(pages) == []


def test_the_link_check_still_flags_a_missing_target() -> None:
    pages = {"a/index.md": "[x](missing.md) `[y](gone.md)`\n"}
    assert build_site.broken_links(pages) == ["a/index.md: broken link missing.md"]


def test_the_link_check_does_not_follow_a_destination_across_table_rows() -> None:
    pages = {"plugins/packs/p/index.md": CROSS_ROW, "plugins/protocol.md": "# P\n"}
    assert build_site.broken_links(pages) == []


def test_starlight_keeps_code_and_rewrites_real_links(tmp_path: Path) -> None:
    body = "# T\n\n`[a](b.md)`\n\n```\n[c](b.md)\n```\n\n[real](b.md)\n"
    pages = {"s/a.md": body, "s/b.md": "# B\n"}
    starlight.write_site(pages, tmp_path, "v3", "https://e.test", "/")
    page = (tmp_path / "src/content/docs/v3/s/a.md").read_text()
    assert "`[a](b.md)`" in page
    assert "```\n[c](b.md)\n```" in page
    assert "[real](../b/)" in page


def test_starlight_leaves_the_cross_row_attack_text_unchanged(tmp_path: Path) -> None:
    pages = {
        "plugins/packs/p/index.md": f"# P\n\n{CROSS_ROW}",
        "plugins/protocol.md": "# Protocol\n",
    }
    starlight.write_site(pages, tmp_path, "v3", "https://e.test", "/")
    page = (tmp_path / "src/content/docs/v3/plugins/packs/p/index.md").read_text()
    assert page.endswith(f"\n\n{CROSS_ROW}")
    parser = MarkdownIt("commonmark", {"html": True}).enable("table")
    tokens = [t for block in parser.parse(page) for t in (block.children or [block])]
    assert not [
        t for t in tokens if t.type.startswith("html") and "script" in t.content
    ]
    assert any(t.type == "code_inline" and "<script>" in t.content for t in tokens)
