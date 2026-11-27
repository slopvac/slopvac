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


def test_refresh_keeps_a_stored_corpus_and_appends_the_new_rule_examples(tmp_path) -> None:
    import json

    stored = ["a stored text that no source produces", "another stored text"]
    (tmp_path / "pattern-parity.json").write_text(
        json.dumps({"regex": "0", "unicode": "0", "texts": stored, "spans": {}, "inert": {}}),
        encoding="utf-8",
    )
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--refresh", "--out", str(tmp_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
    texts = json.loads((tmp_path / "pattern-parity.json").read_text(encoding="utf-8"))[
        "texts"
    ]
    assert texts[:2] == stored
    assert len(texts) > 2
    assert len(texts) == len(set(texts))
