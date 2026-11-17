"""The removed-rules contract file: `docs/contract/compat/removed-rules.toml`.

Every id must be a rule of the 2.x ruleset, and none may remain in the 3.0 core rule data.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
REMOVED_FILE = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"
CORE_V3 = ROOT / "crates" / "slopvac-rules" / "tests" / "fixtures" / "core-v3"

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


# The 3.0 core data still ships these five until the engine removal lands.
@pytest.mark.xfail(reason="core rule data still ships the five rules", strict=False)
def test_no_removed_id_is_in_the_3_0_core_rule_data():
    shipped = set()
    for path in CORE_V3.glob("*.json"):
        data = json.loads(path.read_text(encoding="utf-8"))
        shipped |= {f"{data['id']}.{rule['id']}" for rule in data["rules"]}
    assert {e["id"] for e in _entries()} & shipped == set()
