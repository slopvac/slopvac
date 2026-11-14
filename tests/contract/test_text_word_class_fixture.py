"""Fail when the word-class differential fixture drifts from Python's `regex` module.

`crates/slopvac-text/tests/fixtures/word_class_differential.json` stores generated patterns,
Unicode edge-case texts and the spans Python finds. If the `regex` module changes how it
reads `\w`, `\W`, `\b` or `\B`, this test fails until the fixture is regenerated and the Rust
rewrite is brought in line.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-text/tests/fixtures/generate_word_class.py"
)


def test_word_class_fixture_is_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
