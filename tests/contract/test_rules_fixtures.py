"""Fail when the slopvac-rules fixtures drift from the Python reference.

`crates/slopvac-rules/tests/fixtures` holds the converted core rules (`core-v3/`) and the
spans Python's `regex` module finds in a frozen corpus (`pattern-parity.json`). If a 2.x rule
or `engine.rule_pattern` changes, this test fails until the fixtures are regenerated, and the
Rust parity test then tells whether the PCRE2 matcher still agrees.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-rules/tests/fixtures/generate.py"
)


def test_rules_fixtures_are_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout


def _generate_module():
    import importlib.util

    spec = importlib.util.spec_from_file_location("rules_fixtures_generate", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_merge_corpus_keeps_every_stored_text_in_order_and_appends_new_ones() -> None:
    merge = _generate_module().merge_corpus
    stored = ["b", "a", "stale"]
    built = ["a", "new", "b", "new"]
    assert merge(stored, built) == ["b", "a", "stale", "new"]
