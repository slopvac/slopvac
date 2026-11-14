"""Regenerate reference.json from the Python reference implementation.

Run from the repository root:

    mise exec -- uv run --extra dev python crates/slopvac-text/tests/fixtures/generate.py

The Rust tests read reference.json, so the expected values come from the Python
implementation (`slopvac.analyze`), not from the Rust code. Sentence offsets are
converted from code points to UTF-8 bytes.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from slopvac.analyze import classify_text_type, count_words, split_sentences

HERE = Path(__file__).parent
ROOT = HERE.parents[3]

CURATED = [
    "Open the file.",
    "Do not remove the cover. Make sure the power is off.",
    "WARNING: Do not touch the contacts.",
    "WARNING: High voltage is present inside the unit.",
    "IMPORTANT: Restart the service after the change.",
    "NOTE: The cache is cleared on restart.",
    "> **CAUTION** Remove the battery first.",
    "Tip: use the --force flag.",
    "1. Open the valve.",
    "Step 2.1: Close the valve slowly.",
    "(a) Remove the cap.",
    "Deploy scripts live in the ops folder.",
    "Deploy the 3 scripts live.",
    "Run the install script.",
    "Set up the server. Then log in.",
    "To start the engine, press the button.",
    "Remember to check the logs.",
    "Never reuse the old key.",
    "Don't panic.",
    "You should restart the node.",
    "The API returned an error.",
    "Amazon Web Services announced a new region.",
    "The U.S. government announced it. It was big.",
    "Use e.g. a hammer. Then rest.",
    "He works at Acme Inc. and likes it.",
    "The U.S. Army uses it.",
    "Wait 10 ms then retry. The delay is 512 MiB per second.",
    "Set the temperature to 20 degrees Celsius.",
    'He said "stop right there" and left. She stayed.',
    "Call foo.bar() or http://example.com/a/b now.",
    "Edit /etc/hosts and ./config/app.toml first.",
    "Export $HOME and pass --verbose to the tool.",
    "See fig. 3 for details. The result is clear.",
    "Version 1.2.3 is out. Upgrade now.",
    "It costs 3.5 dollars. That is cheap.",
    "Is it ready? Yes! It is.",
    "The twenty-one-year-old student's book is here.",
    "Café naïve résumé coöperate. 日本語のテキスト。",
    "Emoji 😀 and text. Next sentence here.",
    "Do the following:\n- Open the file.\n- Close the file.\n- Save it.",
    "The steps are:\n1. Open it\n2. Close it",
    "First line wraps\nonto the second line. Then ends.",
    "Wait (for ten minutes) before you go. Then go.",
    "`code span` is skipped. Next one.",
    "A. B. C. initials here. Next.",
    "   Leading space sentence. Another one.",
    "Trailing space sentence.   ",
    "",
    "   ",
    "...",
    "Hello",
    "x1 y22 and abc3def are mixed identifiers.",
    "Go to Section 4 and read No. 5 carefully.",
    "Log in. Sign out. Shut down. Power off the unit.",
    "Turn off the fan and back up the data.",
    "Please do not use the tool. Please use another.",
    "Hold on. Leave the room.",
    "The United States of America and the Republic of France met.",
    "Don’t use curly quotes’ here; it’s fine.",
    "“Quoted words here” and ‘single quoted’ too.",
    "Rules: run tests; fix bugs; ship it.",
    "aः10 degrees Celsius is hot.",
    "Uſe scripts live.",
    "Deploy scripts live here.",
    "Add ¼ cup of water. Then stir.",
    "Add ² more. Then stir.",
    "One.\r\nTwo.\r\nThree",
    "Do the following:\r\n- Open the file.\r\n- Close the file.",
    "First line wraps\r\nonto the second line. Then ends.",
    "A\x1cB. Next sentence.",
    "Check it. \u00b2 more prose follows.",
    "Check it. \u00bc more prose follows.",
    "One. \u1369 item.",
    "One. \U00011136 item.",
    "Deploy \ufb06arts here.",
    "Wait \u203f10 degrees Celsius.",
    "Wait\u200d10 degrees Celsius.",
    "We asked the U.S. It was late.",
    "We asked the U.S. It\u0301 was late.",
]


def paragraphs(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    out = []
    for block in re.split(r"\n\s*\n", text):
        block = block.strip()
        if block and not block.startswith(("#", "|", "---", "<")):
            out.append(block)
    return out


def case(text: str) -> dict:
    def byte(index: int) -> int:
        return len(text[:index].encode("utf-8"))

    return {
        "text": text,
        "words": count_words(text),
        "text_type": classify_text_type(text).value,
        "sentences": [
            {
                "text": s.text,
                "start": byte(s.start),
                "end": byte(s.end),
                "word_count": s.word_count,
                "text_type": s.text_type.value,
            }
            for s in split_sentences(text, 1)
        ],
    }


def build() -> str:
    inputs = list(CURATED)
    for name in (
        "README.md",
        "packages/slopvac-lint/docs/metrics.md",
        "docs/design/slopvac-3.md",
    ):
        path = ROOT / name
        if not path.exists():
            raise SystemExit(f"missing fixture source: {name}")
        inputs.extend(paragraphs(path)[:40])
    return dump([case(t) for t in inputs])


def dump(cases: list[dict]) -> str:
    return json.dumps(cases, ensure_ascii=False, indent=1) + "\n"


def main() -> None:
    import sys

    target = HERE / "reference.json"
    if "--check" in sys.argv:
        # The stored inputs are frozen; only the expected values must track Python.
        stored = json.loads(target.read_text(encoding="utf-8"))
        fresh = dump([case(c["text"]) for c in stored])
        if target.read_text(encoding="utf-8") != fresh:
            raise SystemExit(
                "reference.json is stale; regenerate with generate.py and commit it"
            )
        return
    text = build()
    target.write_text(text, encoding="utf-8")
    print(f"{text.count(chr(10))} lines written")


if __name__ == "__main__":
    main()
