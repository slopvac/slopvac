"""Provider and engine output must be redacted before it is stored or raised."""

from __future__ import annotations

import json
import shlex
import sys

import pytest

from evals import diagrams, models, opencode_model, runner, twins

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


def test_a_successful_model_answer_is_not_rewritten_in_memory():
    """The scorer reads the answer as written; only what is stored is redacted."""
    reply = models.command_ask(writes("stdout", f"Answer. {LEAK}"), attempts=1)("x")
    assert reply.text == f"Answer. {LEAK}"


REVIEW_TASK = {
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
ROSTER = [{"name": "m", "family": "f", "tier": "t", "command": "unused"}]


def test_a_successful_model_answer_is_journaled_without_credentials(tmp_path):
    task, roster = REVIEW_TASK, ROSTER
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
        "Set max_tokens = 4096 and a token budget of 63.",
        "The token budget is 63.\nSecret sauce stays optional.",
        "See /Users/docs/policy.md for a 40-character-long-identifier-name-here.",
    ],
)
def test_ordinary_model_text_is_stored_unchanged(answer):
    reply = models.command_ask(writes("stdout", answer), attempts=1)("x")
    assert reply.text == answer


def crossing_the_boundary(limit: int) -> str:
    """Filler, then a key whose first 8 characters fall inside `limit`."""
    return ("x " * limit)[: limit - 8] + FAKE_KEY + " tail"


def test_a_credential_cut_by_the_stderr_limit_is_still_redacted():
    command = writes("stderr", crossing_the_boundary(300), code=3)
    with pytest.raises(models.CallFailed) as caught:
        models.command_ask(command, attempts=1)("x")
    assert FAKE_KEY[:8] not in str(caught.value)


def test_a_credential_cut_by_the_error_event_limit_is_still_redacted():
    event = json.dumps(
        {"type": "error", "error": {"message": crossing_the_boundary(300)}}
    )
    with pytest.raises(opencode_model.ModelCallError) as caught:
        opencode_model.parse_events(event)
    assert FAKE_KEY[:8] not in str(caught.value)


SCHEME_LEAKS = [
    ("Authorization: Bearer abc123def", "abc123def"),
    ("curl -H 'Authorization: Bearer lowercaseonly'", "lowercaseonly"),
    ('{"Authorization": "Basic dXNlcjpwYXNz"}', "dXNlcjpwYXNz"),
    ('{"password": "ab\\"cd-SECRETTAIL\\"ef"}', "SECRETTAIL"),
]


@pytest.mark.parametrize(("leak", "gone"), SCHEME_LEAKS)
def test_a_twin_with_a_standalone_scheme_is_returned_without_it(leak, gone):
    text = twins.run_provider(writes("stdout", f"A twin. {leak}"), "prompt", 30)
    assert gone not in text


@pytest.mark.parametrize(("leak", "gone"), SCHEME_LEAKS)
def test_a_journaled_answer_with_a_standalone_scheme_is_stored_without_it(
    tmp_path, leak, gone
):
    path = tmp_path / "run.journal.jsonl"
    models.execute(
        [REVIEW_TASK],
        ROSTER,
        3,
        lambda text: [],
        ask_for=lambda m: models.command_ask(writes("stdout", f"[] {leak}")),
        journal=models.Journal(path),
    )
    assert gone not in path.read_text("utf-8")


BODY = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7FAKEFAKEFAKE"


def pem(kind: str, complete: bool = True) -> str:
    block = f"-----BEGIN {kind}-----\n{BODY}\n{BODY}\n"
    return block + (f"-----END {kind}-----" if complete else "")


def stored_answer(tmp_path, text: str) -> str:
    path = tmp_path / "pem.journal.jsonl"
    models.Journal(path).record("job", text, None, 1)
    return json.loads(path.read_text("utf-8"))["answer"]


@pytest.mark.parametrize(
    "kind",
    ["PRIVATE KEY", "RSA PRIVATE KEY", "OPENSSH PRIVATE KEY", "ENCRYPTED PRIVATE KEY"],
)
def test_a_complete_private_key_is_removed_and_the_text_around_it_survives(
    tmp_path, kind
):
    stored = stored_answer(tmp_path, f"Before the key.\n{pem(kind)}\nAfter the key.")
    assert BODY not in stored
    assert "BEGIN" not in stored
    assert stored.startswith("Before the key.\n")
    assert stored.endswith("\nAfter the key.")


def test_a_truncated_private_key_is_removed_and_the_text_before_it_survives(tmp_path):
    stored = stored_answer(
        tmp_path, f"Before the key.\n{pem('PRIVATE KEY', complete=False)}"
    )
    assert BODY not in stored
    assert stored.startswith("Before the key.\n")


@pytest.mark.parametrize("complete", [True, False], ids=["complete", "truncated"])
def test_a_private_key_in_a_journaled_answer_is_stored_without_its_body(
    tmp_path, complete
):
    path = tmp_path / "run.journal.jsonl"
    models.execute(
        [REVIEW_TASK],
        ROSTER,
        3,
        lambda text: [],
        ask_for=lambda m: models.command_ask(
            writes("stdout", f"[] {pem('PRIVATE KEY', complete)}")
        ),
        journal=models.Journal(path),
    )
    stored = path.read_text("utf-8")
    assert BODY not in stored
    assert "BEGIN PRIVATE KEY" not in stored


# --- sinks of successful output: stored reasons, matched text, adapter answers ------


def report_engine(report: dict) -> runner.Engine:
    script = f"import sys; sys.stdout.write({json.dumps(report)!r})"
    return runner.Engine("fake", (sys.executable, "-c", script), runner.REPO_ROOT)


def finding(matched: str) -> dict:
    return {
        "rule_id": "r.one",
        "line": 1,
        "severity": "warning",
        "matched_text": matched,
    }


def test_an_unchecked_note_with_a_credential_is_stored_without_it():
    report = {
        "documents": [
            {"unchecked": [f"rule r.two could not run: {LEAK}"], "findings": []}
        ]
    }
    with pytest.raises(models.LintIncomplete) as caught:
        models.make_linter(report_engine(report))("text")
    assert_clean(str(caught.value))
    assert "rule r.two could not run" in str(caught.value)


def test_engine_matched_text_with_a_credential_is_stored_without_it():
    report = {"documents": [{"findings": [finding(f"use {LEAK} here")]}]}
    found = models.make_linter(report_engine(report))("text")
    assert_clean(json.dumps(found))
    assert found[0]["text"].startswith("use ")


def test_ordinary_matched_text_is_unchanged():
    report = {
        "documents": [{"findings": [finding("delve into the max_tokens setting")]}]
    }
    found = models.make_linter(report_engine(report))("text")
    assert found[0]["text"] == "delve into the max_tokens setting"


def test_diagram_matched_text_with_a_credential_is_stored_without_it():
    labels = {"s": {"labels": []}}
    findings = {
        "s": [
            {"rule": "diagram.note-long", "line": 2, "matched": f"note {LEAK}"},
        ]
    }
    counts = diagrams.score(labels, findings)
    stored = counts["diagram.note-long"]["false_positives"]
    assert len(stored) == 1
    assert_clean(json.dumps(stored))


def test_diagram_matched_text_still_pairs_with_its_label():
    labels = {"s": {"labels": [{"rule": "diagram.note-long", "text": "long note"}]}}
    findings = {
        "s": [{"rule": "diagram.note-long", "line": 2, "matched": "a long note"}]
    }
    assert diagrams.score(labels, findings)["diagram.note-long"]["matched"] == 1


def test_adapter_answers_are_returned_without_credentials():
    events = [
        {"type": "text", "part": {"text": "Answer with AKIAFAKE"}},
        {"type": "text", "part": {"text": "FAKEFAKE0000 inside."}},
    ]
    stream = "\n".join(json.dumps(e) for e in events)
    text = opencode_model.parse_events(stream)
    assert_clean(text)
    assert text.startswith("Answer with")


def test_ordinary_adapter_answers_are_unchanged():
    stream = json.dumps({"type": "text", "part": {"text": "Set max_tokens = 4096."}})
    assert opencode_model.parse_events(stream) == "Set max_tokens = 4096."


CAMEL_JSON = (
    '{"sessionToken": "FAKEsess1234", "accessToken": "FAKEacc5678", '
    '"refreshToken": "FAKErefr9012", "SessionToken": "FAKEcap3456", "limit": 63}'
)
CAMEL_VALUES = ["FAKEsess1234", "FAKEacc5678", "FAKErefr9012", "FAKEcap3456"]


def test_camel_case_token_fields_in_adapter_json_lose_their_values():
    stream = json.dumps({"type": "text", "part": {"text": f"Creds: {CAMEL_JSON}"}})
    text = opencode_model.parse_events(stream)
    assert not any(v in text for v in CAMEL_VALUES)
    assert '"limit": 63' in text


def test_camel_case_token_fields_in_a_journaled_answer_lose_their_values(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    models.execute(
        [REVIEW_TASK],
        ROSTER,
        3,
        lambda text: [],
        ask_for=lambda m: models.command_ask(writes("stdout", f"[] {CAMEL_JSON}")),
        journal=models.Journal(path),
    )
    stored = path.read_text("utf-8")
    assert not any(v in stored for v in CAMEL_VALUES)
