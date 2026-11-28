"""The no-hand-json allow list can only shrink, and it matches what the rule finds."""

from __future__ import annotations

import shutil
import sys
from collections import Counter
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import check_no_hand_json as gate  # noqa: E402

needs_ast_grep = pytest.mark.skipif(
    shutil.which("ast-grep") is None, reason="ast-grep is not on PATH"
)

TEMP = "TEMPORARY(slopvac-8l4.12.88)"
ENTRIES = [
    gate.Entry("crates/a/src/lib.rs", "schema-generation", 3),
    gate.Entry("crates/b/src/lib.rs", TEMP, 2),
]
TOTALS = {"schema-generation": 3, TEMP: 2}
FOUND = Counter({"crates/a/src/lib.rs": 3, "crates/b/src/lib.rs": 2})


def test_matching_list_has_no_problems() -> None:
    assert gate.problems(ENTRIES, TOTALS, FOUND) == []


@pytest.mark.parametrize(
    ("found", "expected"),
    [
        pytest.param(
            Counter({"crates/a/src/lib.rs": 3}),
            "crates/b/src/lib.rs: no finding left",
            id="all-sites-removed-entry-kept",
        ),
        pytest.param(
            Counter({"crates/a/src/lib.rs": 3, "crates/b/src/lib.rs": 1}),
            "crates/b/src/lib.rs: pinned 2 but the rule finds 1",
            id="some-sites-removed-count-kept",
        ),
        pytest.param(
            Counter({"crates/a/src/lib.rs": 4, "crates/b/src/lib.rs": 2}),
            "crates/a/src/lib.rs: pinned 3 but the rule finds 4",
            id="site-added",
        ),
    ],
)
def test_count_drift_is_reported(found: Counter[str], expected: str) -> None:
    assert any(expected in error for error in gate.problems(ENTRIES, TOTALS, found))


def test_category_total_must_match_entries() -> None:
    errors = gate.problems(ENTRIES, {**TOTALS, "schema-generation": 4}, FOUND)
    assert errors == ["total schema-generation: declared 4, entries sum to 3"]


@pytest.mark.parametrize("category", ["TEMPORARY", "TEMPORARY(8l4.3.85)", "misc"])
def test_category_must_be_permanent_or_name_a_bead(category: str) -> None:
    entries = [gate.Entry("crates/a/src/lib.rs", category, 1)]
    errors = gate.problems(entries, {category: 1}, Counter({"crates/a/src/lib.rs": 1}))
    assert any("unknown category" in error for error in errors)


def test_stripping_removes_only_annotated_entries() -> None:
    text = (
        "ignores:\n"
        '  - "**/tests/**"\n'
        "  - crates/a/src/lib.rs # allow digest-input 1\n"
        "  # a comment\n"
        "rule: {}\n"
    )
    assert gate.without_allow_list(text) == (
        'ignores:\n  - "**/tests/**"\n  # a comment\nrule: {}\n'
    )


@needs_ast_grep
def test_the_repository_allow_list_matches_the_rule() -> None:
    assert gate.check(ROOT) == []


def _scratch_root(tmp_path: Path, annotation: str, source: str) -> Path:
    rule = (ROOT / gate.RULE).read_text()
    head, _, tail = rule.partition("ignores:\n")
    kept = [ln for ln in tail.splitlines(keepends=True) if not gate.ENTRY.match(ln)]
    first_rule_line = next(i for i, ln in enumerate(kept) if ln.startswith("rule:"))
    entry = f"  - crates/a/src/lib.rs # allow {annotation}\n"
    text = head + "ignores:\n" + entry + "".join(kept[first_rule_line:])
    text = "\n".join(
        ln for ln in text.splitlines() if not ln.startswith("# total ")
    )
    category = annotation.split()[0]
    text = "# total " + category + " " + annotation.split()[1] + "\n" + text + "\n"
    (tmp_path / "ast-grep" / "rules").mkdir(parents=True)
    (tmp_path / gate.RULE).write_text(text)
    src = tmp_path / "crates" / "a" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text(source)
    return tmp_path


@needs_ast_grep
@pytest.mark.parametrize(
    ("annotation", "source", "expected"),
    [
        pytest.param("digest-input 2", "fn f() { json!(1); json!(2); }\n", [], id="in-step"),
        pytest.param(
            "digest-input 2",
            "fn f() { json!(1); }\n",
            ["pinned 2 but the rule finds 1"],
            id="one-site-removed",
        ),
        pytest.param(
            "digest-input 1",
            "fn f() {}\n",
            ["no finding left, delete the entry"],
            id="every-site-removed",
        ),
    ],
)
def test_real_scan_enforces_the_pinned_count(
    tmp_path: Path, annotation: str, source: str, expected: list[str]
) -> None:
    root = _scratch_root(tmp_path, annotation, source)
    errors = gate.check(root)
    assert len(errors) == len(expected)
    for needle, error in zip(expected, errors, strict=True):
        assert needle in error
