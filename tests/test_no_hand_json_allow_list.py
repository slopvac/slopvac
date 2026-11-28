"""The no-hand-json allow list is closed, matches what the rule finds and can only shrink.

Tests that run the rule need ast-grep. The Rust static job in CI has it and sets
REQUIRE_AST_GREP=1, so there a missing binary fails the test instead of skipping it.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import check_no_hand_json as gate  # noqa: E402

TEMP = "TEMPORARY(slopvac-8l4.12.88)"
REAL = (ROOT / gate.RULE).read_text()
GLOBS = [
    '  - "**/tests/**" # exempt glob',
    '  - "**/benches/**" # exempt glob',
    '  - "**/examples/**" # exempt glob',
    '  - "**/tests.rs" # exempt glob',
]


@pytest.fixture
def ast_grep() -> None:
    if shutil.which("ast-grep") is None:
        if os.environ.get("REQUIRE_AST_GREP"):
            pytest.fail("ast-grep is not on PATH but REQUIRE_AST_GREP is set")
        pytest.skip("ast-grep is not on PATH")


def rule_text(entries: list[str], totals: dict[str, int], extra: list[str] | None = None) -> str:
    """The real rule with its ignore list and total lines replaced."""
    head, _, tail = REAL.partition("ignores:\n")
    head = "\n".join(ln for ln in head.splitlines() if not ln.startswith("# total "))
    rule = tail[tail.index("\nrule:\n") + 1 :]
    lines = [f"# total {category} {count}" for category, count in totals.items()]
    ignores = [*GLOBS, *entries, *(extra or [])]
    return "\n".join([head, *lines, "ignores:", *ignores, rule])


def entry(category: str, count: int, path: str = "crates/a/src/lib.rs") -> str:
    return f"  - {path} # allow {category} {count}"


def scratch(tmp_path: Path, text: str, source: str) -> Path:
    (tmp_path / "ast-grep" / "rules").mkdir(parents=True)
    (tmp_path / gate.RULE).write_text(text)
    src = tmp_path / "crates" / "a" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text(source)
    return tmp_path


def git(root: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        cwd=root,
        check=True,
        capture_output=True,
    )


def committed(tmp_path: Path, text: str, source: str) -> Path:
    root = scratch(tmp_path, text, source)
    git(root, "init", "-q")
    git(root, "add", ".")
    git(root, "commit", "-q", "-m", "base", "--no-gpg-sign")
    return root


ONE = "fn f() { json!(1); }\n"
TWO = "fn f() { json!(1); json!(2); }\n"
BASE_TEXT = rule_text([entry(TEMP, 1)], {TEMP: 1})


# --- the file as written in the repository -------------------------------------------------


def test_the_repository_list_is_well_formed() -> None:
    assert gate.parse(REAL).errors == []


def test_the_repository_allow_list_matches_the_rule(ast_grep: None) -> None:
    assert gate.check(ROOT) == []


# --- strict parsing (no ast-grep needed) ----------------------------------------------------


@pytest.mark.parametrize(
    ("extra", "totals", "expected"),
    [
        pytest.param(
            ["  - crates/a/src/lib.rs"], {}, "needs '# allow", id="unannotated-ignore"
        ),
        pytest.param(
            ["  - crates/a/src/lib.rs # nothing to see"], {}, "needs '# allow", id="odd-note"
        ),
        pytest.param(
            ["  - crates/a/src/lib.rs # allow misc 1"],
            {"misc": 1},
            "unknown category",
            id="unknown-category",
        ),
        pytest.param(
            ["  - crates/a/src/lib.rs # allow TEMPORARY 1"],
            {"TEMPORARY": 1},
            "unknown category",
            id="temporary-without-bead",
        ),
        pytest.param(
            ["  - crates/a/src/lib.rs # allow digest-input 1"],
            {"digest-input": 1},
            None,
            id="valid-entry",
        ),
    ],
)
def test_parse_rejects_an_ignore_without_a_valid_annotation(
    extra: list[str], totals: dict[str, int], expected: str | None
) -> None:
    errors = gate.parse(rule_text([], totals, extra)).errors
    if expected is None:
        assert errors == []
    else:
        assert any(expected in error for error in errors)


def test_parse_rejects_a_duplicate_total() -> None:
    text = rule_text([entry("digest-input", 1)], {"digest-input": 1}).replace(
        "ignores:", "# total digest-input 1\nignores:"
    )
    assert any("duplicate total" in e for e in gate.parse(text).errors)


def test_parse_rejects_a_malformed_total() -> None:
    text = rule_text([], {}).replace("ignores:", "# total digest-input many\nignores:")
    assert any("malformed total" in e for e in gate.parse(text).errors)


def test_file_problems_report_a_missing_total_and_a_wrong_sum() -> None:
    rule = gate.parse(rule_text([entry("digest-input", 2), entry(TEMP, 1, "crates/b/src/lib.rs")], {"digest-input": 3}))
    errors = gate.file_problems(rule, Counter({"crates/a/src/lib.rs": 2, "crates/b/src/lib.rs": 1}))
    assert errors == [
        "total TEMPORARY(slopvac-8l4.12.88): declared None, entries sum to 1",
        "total digest-input: declared 3, entries sum to 2",
    ]


def test_a_glob_outside_the_fixed_set_is_rejected() -> None:
    rule = gate.parse(rule_text([], {}, ['  - "crates/**" # exempt glob']))
    assert any("not one of the fixed test globs" in e for e in gate.file_problems(rule, Counter()))


def test_an_exempt_test_module_must_be_declared_under_cfg_test(tmp_path: Path) -> None:
    src = tmp_path / "crates" / "a" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text("#[cfg(test)]\nmod declared;\nmod open;\n")
    assert gate.declares_test_module(tmp_path, "crates/a/src/declared.rs")
    assert not gate.declares_test_module(tmp_path, "crates/a/src/open.rs")
    assert not gate.declares_test_module(tmp_path, "crates/a/src/missing.rs")


def test_a_tests_rs_file_must_be_a_cfg_test_module(tmp_path: Path) -> None:
    src = tmp_path / "crates" / "a" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text("mod tests;\n")
    (src / "tests.rs").write_text("fn f() {}\n")
    rule = gate.parse(rule_text([], {}))
    assert any("tests.rs" in e for e in gate.exempt_problems(tmp_path, rule))


# --- growth against a base (no ast-grep needed) ---------------------------------------------


def growth_errors(base: str, head: str, label: bool = False) -> list[str]:
    return gate.growth_problems(gate.parse(base), gate.parse(head), label)


def test_raising_a_pin_and_its_total_is_growth() -> None:
    base = rule_text([entry("digest-input", 1)], {"digest-input": 1})
    head = rule_text([entry("digest-input", 2)], {"digest-input": 2})
    errors = growth_errors(base, head)
    assert len(errors) == 2
    assert "count raised 1 -> 2" in errors[0]
    assert "total digest-input: raised 1 -> 2" in errors[1]


def test_a_new_file_entry_is_growth() -> None:
    base = rule_text([entry("digest-input", 1)], {"digest-input": 1})
    head = rule_text(
        [entry("digest-input", 1), entry("digest-input", 1, "crates/b/src/lib.rs")],
        {"digest-input": 2},
    )
    assert any("crates/b/src/lib.rs: new digest-input entry" in e for e in growth_errors(base, head))


def test_a_new_exempt_module_is_growth() -> None:
    base = rule_text([], {})
    head = rule_text([], {}, ["  - crates/a/src/x.rs # exempt test-module"])
    assert any("new exempt test module" in e for e in growth_errors(base, head))


def test_moving_a_file_from_temporary_to_permanent_is_growth() -> None:
    base = rule_text([entry(TEMP, 1)], {TEMP: 1})
    head = rule_text([entry("digest-input", 1)], {"digest-input": 1})
    assert any("category" in e for e in growth_errors(base, head))


def test_permanent_growth_passes_with_the_label() -> None:
    base = rule_text([entry("digest-input", 1)], {"digest-input": 1})
    head = rule_text([entry("digest-input", 2)], {"digest-input": 2})
    assert growth_errors(base, head, label=True) == []


@pytest.mark.parametrize("label", [False, True])
def test_temporary_growth_never_passes(label: bool) -> None:
    head = rule_text([entry(TEMP, 2)], {TEMP: 2})
    errors = growth_errors(BASE_TEXT, head, label)
    assert errors
    assert all(e.startswith("TEMPORARY growth is never allowed") for e in errors)


@pytest.mark.parametrize("label", [False, True])
def test_a_new_temporary_entry_never_passes(label: bool) -> None:
    head = rule_text(
        [entry(TEMP, 1), entry(TEMP, 1, "crates/b/src/lib.rs")], {TEMP: 2}
    )
    assert growth_errors(BASE_TEXT, head, label)


def test_shrinking_passes() -> None:
    head = rule_text([], {})
    assert growth_errors(BASE_TEXT, head) == []


@pytest.mark.parametrize(
    ("labels", "expected"),
    [
        ('["no-hand-json-growth", "ci"]', True),
        ('["ci"]', False),
        ("[]", False),
        ("not json", False),
        ('{"name": "no-hand-json-growth"}', False),
        (None, False),
    ],
)
def test_the_label_is_read_from_a_json_list_of_names(labels: str | None, expected: bool) -> None:
    assert gate.has_label(labels) is expected


# --- the checker boundary, with the rule running (needs ast-grep) -----------------------------


@pytest.mark.parametrize(
    ("pinned", "source", "needle"),
    [
        pytest.param(2, TWO, None, id="in-step"),
        pytest.param(2, ONE, "pinned 2 but the rule finds 1", id="one-site-removed"),
        pytest.param(1, TWO, "pinned 1 but the rule finds 2", id="site-added"),
        pytest.param(1, "fn f() {}\n", "no finding left, delete the entry", id="all-removed"),
    ],
)
def test_the_pinned_count_must_equal_the_findings(
    ast_grep: None, tmp_path: Path, pinned: int, source: str, needle: str | None
) -> None:
    text = rule_text([entry("digest-input", pinned)], {"digest-input": pinned})
    errors = gate.check(scratch(tmp_path, text, source))
    if needle is None:
        assert errors == []
    else:
        assert len(errors) == 1
        assert needle in errors[0]


def test_a_total_that_differs_from_its_entries_fails(ast_grep: None, tmp_path: Path) -> None:
    root = scratch(tmp_path, rule_text([entry("digest-input", 1)], {"digest-input": 2}), ONE)
    assert gate.check(root) == ["total digest-input: declared 2, entries sum to 1"]


def test_raised_pin_with_a_new_match_fails_against_the_base(ast_grep: None, tmp_path: Path) -> None:
    root = committed(tmp_path, rule_text([entry("digest-input", 1)], {"digest-input": 1}), ONE)
    (root / gate.RULE).write_text(rule_text([entry("digest-input", 2)], {"digest-input": 2}))
    (root / "crates/a/src/lib.rs").write_text(TWO)
    errors = gate.check(root, base="HEAD")
    assert any("count raised 1 -> 2" in e for e in errors)
    assert gate.check(root, base="HEAD", labels='["no-hand-json-growth"]') == []


def test_a_new_file_entry_fails_against_the_base(ast_grep: None, tmp_path: Path) -> None:
    root = committed(tmp_path, rule_text([], {}), "fn f() {}\n")
    (root / gate.RULE).write_text(rule_text([entry("digest-input", 1)], {"digest-input": 1}))
    (root / "crates/a/src/lib.rs").write_text(ONE)
    assert any("new digest-input entry" in e for e in gate.check(root, base="HEAD"))


def test_a_raised_temporary_count_fails_even_with_the_label(ast_grep: None, tmp_path: Path) -> None:
    root = committed(tmp_path, BASE_TEXT, ONE)
    (root / gate.RULE).write_text(rule_text([entry(TEMP, 2)], {TEMP: 2}))
    (root / "crates/a/src/lib.rs").write_text(TWO)
    errors = gate.check(root, base="HEAD", labels='["no-hand-json-growth"]')
    assert errors
    assert all("TEMPORARY growth is never allowed" in e for e in errors)


def test_an_unannotated_ignore_fails_the_check(ast_grep: None, tmp_path: Path) -> None:
    root = scratch(tmp_path, rule_text([], {}, ["  - crates/a/src/lib.rs"]), ONE)
    assert any("needs '# allow" in e for e in gate.check(root))


def test_a_base_without_the_rule_has_nothing_to_compare(ast_grep: None, tmp_path: Path) -> None:
    root = scratch(tmp_path, rule_text([entry("digest-input", 1)], {"digest-input": 1}), ONE)
    git(root, "init", "-q")
    (root / "README.md").write_text("x\n")
    git(root, "add", "README.md")
    git(root, "commit", "-q", "-m", "base", "--no-gpg-sign")
    assert gate.check(root, base="HEAD") == []


def test_an_unknown_base_is_an_error_not_a_pass(ast_grep: None, tmp_path: Path) -> None:
    root = committed(tmp_path, rule_text([], {}), "fn f() {}\n")
    with pytest.raises(RuntimeError, match="not a commit"):
        gate.check(root, base="0" * 40)


def test_merge_group_mode_skips_growth_but_keeps_the_file_checks(
    ast_grep: None, tmp_path: Path
) -> None:
    root = committed(tmp_path, rule_text([entry("digest-input", 1)], {"digest-input": 1}), ONE)
    (root / gate.RULE).write_text(rule_text([entry("digest-input", 2)], {"digest-input": 2}))
    (root / "crates/a/src/lib.rs").write_text(TWO)
    assert gate.check(root, base="HEAD")  # the pull request run sees the growth
    assert gate.check(root) == []  # no base: the queue skips only the growth comparison
    (root / gate.RULE).write_text(rule_text([entry("digest-input", 2)], {"digest-input": 3}))
    assert gate.check(root) == ["total digest-input: declared 3, entries sum to 2"]


def test_the_command_line_reads_the_labels_it_is_given(ast_grep: None, tmp_path: Path) -> None:
    root = committed(tmp_path, rule_text([entry("digest-input", 1)], {"digest-input": 1}), ONE)
    (root / gate.RULE).write_text(rule_text([entry("digest-input", 2)], {"digest-input": 2}))
    (root / "crates/a/src/lib.rs").write_text(TWO)
    script = ROOT / "scripts" / "ci" / "check_no_hand_json.py"

    def run(*extra: str) -> int:
        return subprocess.run(
            [sys.executable, str(script), str(root), "--base", "HEAD", *extra],
            capture_output=True,
            check=False,
        ).returncode

    assert run() == 1
    assert run("--labels", json.dumps(["no-hand-json-growth"])) == 0
    assert run("--labels", json.dumps(["other"])) == 1
