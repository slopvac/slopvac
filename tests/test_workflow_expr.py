"""The expression evaluator that workflow routing tests rely on."""

from __future__ import annotations

import pytest

from tests.workflow_expr import evaluate, run_jobs


@pytest.mark.parametrize(
    ("condition", "context", "expected"),
    [
        ("${{ a.b == 'x' }}", {"a.b": "x"}, True),
        ("a.b == 'x'", {"a.b": "X"}, True),
        ("a.b == 'x'", {"a.b": "y"}, False),
        ("a.b == 'x'", {}, False),
        ("a.b != 'x'", {}, True),
        ("!a", {"a": False}, True),
        ("!a", {"a": ""}, True),
        ("!a", {}, True),
        ("!a", {"a": "false"}, False),
        ("a == 'x' || b == 'y' && c == 'z'", {"a": "x"}, True),
        ("(a == 'x' || b == 'y') && c == 'z'", {"a": "x"}, False),
        ("(a == 'x' || b == 'y') && c == 'z'", {"b": "y", "c": "z"}, True),
        ("a == 'it''s'", {"a": "it's"}, True),
    ],
)
def test_expressions(condition, context, expected):
    assert evaluate(condition, context) is expected


@pytest.mark.parametrize(
    ("condition", "status", "expected"),
    [
        ("!cancelled()", "success", True),
        ("!cancelled()", "cancelled", False),
        ("success()", "failure", False),
        ("failure()", "failure", True),
        ("always()", "cancelled", True),
    ],
)
def test_status_functions(condition, status, expected):
    assert evaluate(condition, {}, status=status) is expected


@pytest.mark.parametrize("bad", ["a ==", "(a", "a b", "nope()", "a $ b"])
def test_malformed_expressions_are_errors(bad):
    with pytest.raises(ValueError):
        evaluate(bad, {})


JOBS = {
    "a": {},
    "b": {"needs": "a"},
    "c": {"needs": ["b"], "if": "${{ !cancelled() }}"},
    "d": {"needs": ["b"], "if": "needs.b.outputs.go == 'yes'"},
}


def test_a_skipped_need_skips_its_dependents_unless_a_status_function_decides():
    skipped_b = {**JOBS, "b": {"needs": "a", "if": "false_name"}}
    assert run_jobs(skipped_b, {}, {}) == {"a", "c"}


def test_a_failed_need_skips_dependents_without_a_status_function():
    assert run_jobs(JOBS, {}, {"b": {"go": "yes"}}, fail=frozenset({"b"})) == {
        "a",
        "b",
        "c",
    }


def test_a_job_reads_outputs_of_its_needs_only():
    assert run_jobs(JOBS, {}, {"b": {"go": "yes"}}) == {"a", "b", "c", "d"}
    assert run_jobs(JOBS, {}, {"a": {"go": "yes"}}) == {"a", "b", "c"}
