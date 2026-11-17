"""Provider and engine output must be redacted before it is stored or raised."""

from __future__ import annotations

import json
import shlex
import sys

import pytest

from evals import diagrams, models, runner, twins

FAKE_KEY = "AKIAFAKEFAKEFAKE0000"
FAKE_SECRET = "FAKEsecretFAKEsecretFAKEsecretFAKEsecret00"
LEAK = f"AWS_ACCESS_KEY_ID={FAKE_KEY} AWS_SECRET_ACCESS_KEY={FAKE_SECRET}"


def python_command(script: str) -> str:
    return shlex.join([sys.executable, "-c", script])


def writes(stream: str, text: str, code: int = 0) -> str:
    return python_command(f"import sys; sys.{stream}.write({text!r}); sys.exit({code})")


def assert_clean(text: str) -> None:
    assert FAKE_KEY not in text
    assert FAKE_SECRET not in text


def engine(stdout: str, stderr: str, code: int) -> runner.Engine:
    script = (
        f"import sys; sys.stdout.write({stdout!r}); "
        f"sys.stderr.write({stderr!r}); sys.exit({code})"
    )
    return runner.Engine("fake", (sys.executable, "-c", script), runner.REPO_ROOT)


@pytest.mark.parametrize(
    ("stdout", "code"),
    [("not json", 0), (json.dumps({"no": "documents"}), 0), ("", 3)],
    ids=["no-report", "report-without-documents", "failed-exit"],
)
def test_engine_errors_do_not_echo_credentials_from_stderr(stdout, code):
    with pytest.raises(runner.EngineError) as caught:
        engine(stdout, LEAK, code).lint([])
    assert_clean(str(caught.value))
    assert "fake exited" in str(caught.value)


def test_diagram_engine_errors_do_not_echo_credentials_from_stderr():
    command = [
        sys.executable,
        "-c",
        f"import sys; sys.stderr.write({LEAK!r}); sys.exit(4)",
    ]
    with pytest.raises(diagrams.DiagramEvalError) as caught:
        diagrams.run_engine(command, [])
    assert_clean(str(caught.value))
    assert "engine exited 4" in str(caught.value)


def test_a_successful_model_answer_is_returned_without_credentials():
    reply = models.command_ask(writes("stdout", f"Answer. {LEAK}"), attempts=1)("x")
    assert_clean(reply.text)
    assert reply.text.startswith("Answer.")


def test_a_successful_model_answer_is_journaled_without_credentials(tmp_path):
    task = {
        "id": "p1",
        "type": "rewrite",
        "role": "reviewer",
        "variant": "planted",
        "doc": "d",
        "genre": "g",
        "prompt": "review",
        "edits": [{"kind": "n", "sentence": "alpha beta gamma delta"}],
        "passage": "alpha beta gamma delta\nalpha beta gamma epsilon",
    }
    roster = [{"name": "m", "family": "f", "tier": "t", "command": "unused"}]
    journal = models.Journal(tmp_path / "run.journal.jsonl")
    out = models.execute(
        [task],
        roster,
        3,
        lambda text: [],
        ask_for=lambda m: models.command_ask(writes("stdout", f"[] {LEAK}")),
        journal=journal,
    )
    assert_clean((tmp_path / "run.journal.jsonl").read_text("utf-8"))
    assert_clean(json.dumps(out))


def test_a_successful_twin_is_returned_without_credentials():
    text = twins.run_provider(writes("stdout", f"A twin. {LEAK}"), "prompt", 30)
    assert_clean(text)
    assert text.startswith("A twin.")


@pytest.mark.parametrize(
    "answer",
    [
        "Cache 40 items now. Use `max_retries` 3 times.",
        "The token budget is 63.\nSecret sauce stays optional.",
        "See /Users/docs/policy.md for a 40-character-long-identifier-name-here.",
    ],
)
def test_ordinary_model_text_is_stored_unchanged(answer):
    reply = models.command_ask(writes("stdout", answer), attempts=1)("x")
    assert reply.text == answer
