"""The removed-rules contract file: `docs/contract/compat/removed-rules.toml`.

Every id must be a rule of the 2.x ruleset.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
REMOVED_FILE = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"

# Worked out from the user decision, not read from the file under test.
EXPECTED = {
    "ste-practices.inconsistent-wording-for-same-step",
    "ste-safety.safety-block-missing-consequence",
    "ste-sentences.vertical-list-item-punctuation",
    "ste-sentences.vertical-list-lead-in-missing-colon",
    "ste-words.inconsistent-term-for-same-thing",
}


def _entries() -> list[dict]:
    return tomllib.loads(REMOVED_FILE.read_text(encoding="utf-8"))["rules"]


def test_the_file_lists_the_five_removed_rules_once_each():
    ids = [entry["id"] for entry in _entries()]
    assert sorted(ids) == sorted(EXPECTED)


def test_each_entry_has_a_one_line_reason_and_a_source():
    for entry in _entries():
        assert set(entry) == {"id", "reason", "source"}, entry["id"]
        assert entry["reason"].strip() and "\n" not in entry["reason"], entry["id"]
        assert entry["source"].strip(), entry["id"]


def test_every_removed_id_is_a_2x_rule():
    from slopvac.rules import load_ruleset

    known = {rule.qualified_id for rule in load_ruleset().all_rules}
    assert {e["id"] for e in _entries()} - known == set()


def test_the_converter_reads_the_contract_file_and_fails_without_it(tmp_path):
    import importlib.util
    import sys

    path = ROOT / "scripts" / "contract" / "convert_rules_v2_to_v3.py"
    spec = importlib.util.spec_from_file_location("convert_removed_probe", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)

    assert module.REMOVED_RULES == EXPECTED
    with pytest.raises(FileNotFoundError):
        module.load_removed_rules(tmp_path / "missing.toml")
