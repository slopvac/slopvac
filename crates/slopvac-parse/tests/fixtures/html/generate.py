"""Regenerate reference.json from the Python reference implementation.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-parse/tests/fixtures/html/generate.py
    mise exec -- uv run --extra dev python crates/slopvac-parse/tests/fixtures/html/generate.py --check

The Rust tests read reference.json, so the expected values come from the Python HTML
projection (`slopvac.analyze._parse_html`, which feeds the vocabulary and metric rules),
not from the Rust code. For each document the fixture lists every whitespace-separated
word of the visible text and the source text the Python map slices for it. The Rust
extractor must produce the same words with the same source text. The inputs stay inside
the HTML that both parsers read alike. See divergences.md for what they read differently.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from slopvac.analyze import _parse_html

HERE = Path(__file__).parent

# (id, source). Every input keeps a space or a block tag wherever a tag sits between two
# words: Python puts a space at every tag boundary, the Rust extractor only at block tags.
CURATED: list[tuple[str, str]] = [
    ("paragraph", "<p>One plain paragraph.</p>\n"),
    ("two_paragraphs_one_line", "<p>First one.</p><p>Second one.</p>\n"),
    ("two_paragraphs_two_lines", "<p>First one.</p>\n<p>Second one.</p>\n"),
    ("wrapped_paragraph", "<p>First line wraps\nonto the second line.</p>\n"),
    ("wrapped_paragraph_crlf", "<p>First line wraps\r\nonto the second line.</p>\r\n"),
    ("multibyte", "<p>Café naïve 日本語のテキスト and emoji 😀 here.</p>\n"),
    ("no_final_newline", "<p>Ends without a newline</p>"),
    ("bare_text", "Words with no tags at all.\n"),
    (
        "headings",
        "<h1>Title here</h1>\n<p>Body text follows.</p>\n<h2>Second heading</h2>\n",
    ),
    ("list", "<ul>\n<li>First item</li>\n<li>Second item</li>\n</ul>\n"),
    ("list_one_line", "<ul><li>First item</li><li>Second item</li></ul>\n"),
    ("unclosed_items", "<ul>\n<li>First item\n<li>Second item\n</ul>\n"),
    (
        "table",
        "<table>\n<tr><th>Name</th><th>Value</th></tr>\n<tr><td>alpha</td><td>beta</td></tr>\n</table>\n",
    ),
    (
        "inline_emphasis",
        "<p>A <em>tagged</em> word and <strong>another</strong> one.</p>\n",
    ),
    (
        "inline_in_cell",
        "<table><tr><td>The <em>leverage</em> is high</td></tr></table>\n",
    ),
    (
        "link",
        '<p>See <a href="https://example.com/a?x=1&amp;y=2">the guide</a> for details.</p>\n',
    ),
    ("attribute_with_angle", '<p title="a > b">Visible text.</p>\n'),
    ("comment", "<p>Visible <!-- hidden words --> text.</p>\n"),
    (
        "comment_multiline",
        "<p>Before</p>\n<!-- hidden\nacross lines -->\n<p>After</p>\n",
    ),
    ("entity_inside_word", "<p>AT&amp;T rocks and a&lt;b work.</p>\n"),
    ("entity_at_word_end", "<p>R&amp;D. Q&amp;A, Tom&amp;</p>\n"),
    ("numeric_entities", "<p>Use a&#169;b and c&#x1F600;d marks.</p>\n"),
    ("named_entities", "<p>caf&eacute; &mdash; &quot;quoted&quot; &copy; 2026</p>\n"),
    ("nbsp", "<p>one&nbsp;two three</p>\n"),
    (
        "script_skipped",
        "<p>Keep this.</p>\n<script>var a = '<b>no</b>'; // &amp;</script>\n<p>And this.</p>\n",
    ),
    ("style_skipped", "<style>p { color: red }</style>\n<p>Styled text.</p>\n"),
    (
        "head_skipped",
        "<html><head><title>Page title</title></head>\n<body>\n<p>Body words.</p>\n</body></html>\n",
    ),
    (
        "pre_skipped",
        "<p>Run this:</p>\n<pre>cargo test\nmake all</pre>\n<p>Then stop.</p>\n",
    ),
    ("inline_code_skipped", "<p>Run <code>cargo test</code> before you push.</p>\n"),
    ("kbd_skipped", "<p>Press <kbd>Ctrl</kbd> to continue.</p>\n"),
    (
        "template_skipped",
        "<template><p>Hidden words.</p></template>\n<p>Shown words.</p>\n",
    ),
    ("noscript_skipped", "<noscript>Enable scripts.</noscript>\n<p>Shown words.</p>\n"),
    ("br_with_spaces", "<p>line one <br> line two</p>\n"),
    (
        "blockquote",
        "<blockquote>\nQuoted line one\nquoted line two\n</blockquote>\n<p>After the quote.</p>\n",
    ),
    ("blank_lines_many", "<p>One.</p>\n\n\n\n<p>Two.</p>\n"),
    ("trailing_spaces", "<p>Line with trailing spaces   \nnext line.</p>\n"),
    ("tabs", "<p>\tIndented\twith\ttabs</p>\n"),
    (
        "full_page",
        '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>T</title>\n'
        "<style>b { x: y }</style>\n</head>\n<body>\n<h1>Guide</h1>\n"
        "<p>Intro text with <code>code</code> and <b>bold</b> parts.</p>\n"
        "<ul>\n<li>Step one</li>\n<li>Step two</li>\n</ul>\n<script>go()</script>\n</body>\n</html>\n",
    ),
]


def words(text: str) -> list[tuple[int, int]]:
    return [(m.start(), m.end()) for m in re.finditer(r"\S+", text)]


def case(case_id: str, source: str) -> dict:
    document = _parse_html("fixture.html", source)
    out: list[dict] = []
    for block in document.blocks:
        projection = block.projection
        for start, end in words(block.text):
            out.append(
                {
                    "text": block.text[start:end],
                    "src": projection.slice_raw(start, end).decode("utf-8"),
                }
            )
    return {"id": case_id, "source": source, "words": out}


def dump(cases: list[dict]) -> str:
    return json.dumps(cases, ensure_ascii=False, indent=1) + "\n"


def main() -> None:
    target = HERE / "reference.json"
    if "--check" in sys.argv:
        # The stored inputs are frozen; only the expected values must track Python.
        stored = json.loads(target.read_text(encoding="utf-8"))
        fresh = dump([case(c["id"], c["source"]) for c in stored])
        if target.read_text(encoding="utf-8") != fresh:
            raise SystemExit(
                "reference.json is stale; regenerate with generate.py and commit it"
            )
        return
    target.write_text(dump([case(i, s) for i, s in CURATED]), encoding="utf-8")
    print(f"{len(CURATED)} cases written")


if __name__ == "__main__":
    main()
