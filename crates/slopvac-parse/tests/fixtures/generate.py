"""Regenerate reference.json from the Python reference implementation.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-parse/tests/fixtures/generate.py
    mise exec -- uv run --extra dev python crates/slopvac-parse/tests/fixtures/generate.py --check

The Rust tests read reference.json, so the expected values come from the Python
projection (`slopvac.projection.project`), not from the Rust code. For each document the
fixture lists every whitespace-separated word of the projected text and the source text
the Python map slices for it. The Rust parser must produce the same words with the same
source text. The inputs stay inside the Markdown that both parsers read alike. See
divergences.md for what they read differently.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from slopvac.projection import project

HERE = Path(__file__).parent

# (id, source). Masked inline code is NUL in Rust and " NUL " in Python, so the words
# are taken with NUL replaced by a space.
CURATED: list[tuple[str, str]] = [
    ("paragraph", "One plain paragraph.\n"),
    ("soft_wrap", "First line wraps\nonto the second line.\n"),
    ("soft_wrap_crlf", "First line wraps\r\nonto the second line.\r\n"),
    ("two_paragraphs", "First paragraph.\n\nSecond paragraph.\n"),
    ("no_final_newline", "Ends without a newline"),
    (
        "multibyte",
        "Café naïve 日本語のテキスト and emoji 😀 here.\n\nNext résumé line.\n",
    ),
    ("heading", "# Title here\n\nBody text follows.\n\n## Second heading\n"),
    ("bullet_list", "- First item\n- Second item\n  continues here\n- Third item\n"),
    ("numbered_list", "1. Open the valve.\n2. Close the valve slowly.\n"),
    ("task_list_text", "* Star item one\n* Star item two\n"),
    ("blockquote", "> Quoted line one\n> quoted line two\n\nAfter the quote.\n"),
    ("quote_with_list", "> - Quoted item\n> - Another item\n"),
    ("strong", "A **strong** word and __another__ one.\n"),
    ("strikethrough", "Some ~~removed~~ text stays.\n"),
    ("link", "See [the guide](https://example.com/a) for details.\n"),
    ("inline_code", "Run `cargo test` before you push.\n"),
    ("fenced_code", "Before.\n\n```python\nx = 1\nprint(x)\n```\n\nAfter.\n"),
    ("tilde_fence", "Before.\n\n~~~\ncode here\n~~~\n\nAfter.\n"),
    ("indented_code", "Before.\n\n    indented code\n    more code\n\nAfter.\n"),
    ("inline_tags", "A <em>tagged</em> word and <br/> a break.\n"),
    ("inline_comment", "Visible <!-- hidden --> text.\n"),
    ("entity_inside_word", "AT&amp;T rocks and a&lt;b work.\n"),
    ("numeric_entity", "Use a&#169;b and c&#x1F600;d marks.\n"),
    (
        "mixed_document",
        "# Guide\n\nIntro text with `code` and **bold**.\n\n- Step one\n- Step two\n\n> Note this.\n\n```sh\nmake\n```\n\nDone.\n",
    ),
    ("blank_lines_many", "One.\n\n\n\nTwo.\n"),
    ("trailing_spaces", "Line with trailing spaces   \nnext line.\n"),
]


def words(text: str) -> list[tuple[int, int]]:
    cleaned = text.replace("\x00", " ")
    return [(m.start(), m.end()) for m in re.finditer(r"\S+", cleaned)]


def case(case_id: str, source: str) -> dict:
    text, pm = project(source)
    cleaned = text.replace("\x00", " ")
    return {
        "id": case_id,
        "source": source,
        "text": text,
        "words": [
            {
                "text": cleaned[start:end],
                "src": pm.slice_raw(start, end).decode("utf-8"),
            }
            for start, end in words(text)
        ],
    }


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
    text = dump([case(i, s) for i, s in CURATED])
    target.write_text(text, encoding="utf-8")
    print(f"{len(CURATED)} cases written")


if __name__ == "__main__":
    main()
