"""The conformance suite, run against good and deliberately broken plugins."""

import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

import pytest
from conftest import EXAMPLE, FIXTURES, SDK_ENV, bounded, script
from slopvac_plugin.conformance import SuiteError, load_declaration, run_conformance
from slopvac_plugin.conformance.cases import CASES, check_edits, handshake
from slopvac_plugin.conformance.session import CaseFailure, CaseSkip, Session

ALL_IDS = [f"P{i}" for i in range(1, 20)]
REPO = Path(__file__).resolve().parents[3]
PACK_SCHEMA = REPO / "schemas" / "v3" / "pack-manifest.schema.json"

ALL_HOOKS_INPUTS = {
    "provider": {},
    "tagger": {"invalid_document": {"id": "bad", "text": "a\u0000b"}},
    "extractor": {"invalid_document": {"id": "bad", "text": "a\u0000b"}},
    "fixer": {"invalid_document": {"id": "bad", "text": "a\u0000b", "findings": []}},
    "scorer": {"invalid_document": {"id": "bad", "text": "a\u0000b", "findings": []}},
}


def statuses(report):
    return {r.id: r.status for r in report.results}


def test_example_plugin_passes_every_applicable_case(quiet_cwd):
    command = [sys.executable, str(EXAMPLE / "plugin.py")]
    inputs = json.loads((EXAMPLE / "conformance-inputs.json").read_text("utf-8"))
    report = run_conformance(
        command,
        declaration=load_declaration(EXAMPLE / "slopvac-pack.toml"),
        inputs=inputs,
        pass_env=SDK_ENV,
    )
    assert report.failed == []
    assert report.exit_code == 0
    # The example declares a tagger only, so the provider, recipe and fixer cases skip.
    skipped = {r.id for r in report.results if r.status == "skip"}
    assert skipped == {"P16", "P17", "P18"}
    assert [r.id for r in report.results] == ALL_IDS


def test_plugin_with_every_hook_kind_passes_all_19_cases(tmp_path, quiet_cwd):
    declaration = {
        "network": True,
        "hooks": [
            {"kind": "tagger", "id": "words"},
            {"kind": "extractor", "id": "lines"},
            {"kind": "reporter", "id": "plain"},
            {"kind": "fixer", "id": "upper"},
            {"kind": "scorer", "id": "count"},
            {"kind": "provider", "id": "echo"},
            {"kind": "recipe", "id": "files"},
        ],
        "features": {"required": [], "optional": ["document-path"]},
    }
    path = tmp_path / "declaration.json"
    path.write_text(json.dumps(declaration), encoding="utf-8")
    report = run_conformance(
        script("all_hooks_plugin.py"),
        declaration=load_declaration(path),
        inputs=ALL_HOOKS_INPUTS,
        pass_env=SDK_ENV,
    )
    assert statuses(report) == dict.fromkeys(ALL_IDS, "pass"), report.to_text()


def test_plugin_with_a_one_document_batch_limit_passes_the_suite(quiet_cwd):
    report = run_conformance(
        script("one_document_plugin.py"), inputs=ALL_HOOKS_INPUTS, pass_env=SDK_ENV
    )
    assert report.failed == [], report.to_text()
    # P7 needs a two-document batch, which this plugin refuses by contract.
    assert statuses(report)["P7"] == "skip"


def test_plugin_written_without_the_sdk_passes(quiet_cwd):
    report = run_conformance(script("raw_plugin.py"))
    assert report.failed == [], report.to_text()


@pytest.mark.parametrize(
    ("mode", "failing"),
    [
        ("no-limit", "P13"),  # reads and answers a body over maxMessageBytes
        ("stdout-noise", "P14"),  # writes a stray line to standard output
        ("bad-span", "P15"),  # span ends past the text
        ("split-char-span", "P15"),  # span ends inside a multi-byte character
        ("exit-zero-without-shutdown", "P11"),  # exit status 0 where 1 is required
        ("no-version-check", "P2"),  # accepts a version range without 1
        ("read-oversized-body", "P13"),  # waits for a body it must not read
        ("float-id", "P1"),  # answers with id 1.0
        ("float-limit", "P1"),  # maxMessageBytes spelled 65536.0
        ("big-init-reply", "P1"),  # an initialize reply over 1 MiB
        ("in-flight-17", "P1"),  # maxInFlight above the host ceiling of 16
        ("in-flight-unsafe", "P1"),  # maxInFlight beyond 2^53-1
    ],
)
def test_suite_fails_a_broken_plugin_on_the_violated_case(mode, failing, quiet_cwd):
    report = run_conformance(script("raw_plugin.py", mode), timeout=5)
    assert statuses(report)[failing] == "fail", report.to_text()
    assert report.exit_code == 1


def test_a_reply_over_the_negotiated_limit_is_a_framing_violation(start):
    s = start(script("raw_plugin.py", "big-reply"))
    handshake(s, max_message_bytes=65536)
    with pytest.raises(CaseFailure, match="exceeds the limit of 65536"):
        s.request(
            "tagger/tag", {"tagger": "main", "documents": [{"id": "d", "text": "t"}]}
        )


def test_a_reply_over_the_pre_initialize_limit_names_the_limit(quiet_cwd):
    report = run_conformance(script("raw_plugin.py", "big-init-reply"), only=["P1"])
    assert "exceeds the limit of 1048576" in report.failed[0].detail


def test_cancel_grace_runs_from_the_cancel_not_from_the_request(quiet_cwd):
    # The plugin ignores the cancel and answers after 3 s: far past the 0.5 s grace.
    report = run_conformance(
        script("raw_plugin.py", "slow-tagger"), only=["P9"], cancel_grace=0.5
    )
    assert statuses(report) == {"P9": "fail"}
    assert "no reply in time" in report.failed[0].detail


def test_cancel_grace_passes_a_plugin_that_answers_at_once(quiet_cwd):
    report = run_conformance(script("raw_plugin.py"), only=["P9"], cancel_grace=0.5)
    assert statuses(report) == {"P9": "pass"}


def test_a_plugin_that_stops_reading_fails_p13_instead_of_hanging(quiet_cwd):
    outcome = []

    def run() -> None:
        outcome.append(
            run_conformance(
                script("raw_plugin.py", "deaf-after-init"),
                only=["P13"],
                timeout=1,
                exit_timeout=1,
            )
        )

    assert bounded(run, 60), "the suite hung on a plugin that never reads"
    assert statuses(outcome[0]) == {"P13": "fail"}


def test_a_failure_names_the_reason(quiet_cwd):
    report = run_conformance(script("raw_plugin.py", "bad-span"), only=["P15"])
    assert "outside" in report.failed[0].detail


def test_missing_declaration_skips_p19_with_a_reason(quiet_cwd):
    report = run_conformance(script("raw_plugin.py"), only=["P19"])
    assert report.results[0].status == "skip"
    assert "--manifest" in report.results[0].detail


def test_p19_fails_a_reply_wider_than_the_declaration(quiet_cwd):
    declaration = {
        "network": False,
        "hooks": [],
        "features": {"required": [], "optional": []},
    }
    report = run_conformance(
        script("raw_plugin.py"), declaration=declaration, only=["P19"]
    )
    assert statuses(report) == {"P19": "fail"}
    assert "tagger" in report.failed[0].detail


def test_command_that_does_not_start_is_incomplete_not_failed(quiet_cwd):
    with pytest.raises(SuiteError):
        run_conformance(["/nonexistent/plugin-binary"])


def cli(*args, cwd):
    return subprocess.run(
        [sys.executable, "-m", "slopvac_plugin.conformance", *args],
        check=False,
        capture_output=True,
        text=True,
        cwd=cwd,
        timeout=120,
    )


def test_cli_exit_0_and_text_report_for_a_passing_plugin(tmp_path):
    done = cli("--only", "P1,P3", "--", *script("raw_plugin.py"), cwd=tmp_path)
    assert done.returncode == 0, done.stdout + done.stderr
    assert done.stdout.splitlines()[-1] == "2 passed, 0 failed, 0 skipped"


def test_cli_exit_1_and_json_report_for_a_failing_plugin(tmp_path):
    done = cli(
        "--only",
        "P11",
        "--format",
        "json",
        "--",
        *script("raw_plugin.py", "exit-zero-without-shutdown"),
        cwd=tmp_path,
    )
    assert done.returncode == 1
    report = json.loads(done.stdout)
    assert report["ok"] is False
    assert [(r["id"], r["status"]) for r in report["results"]] == [("P11", "fail")]


def test_cli_exit_2_when_the_command_does_not_start(tmp_path):
    done = cli("--", "/nonexistent/plugin-binary", cwd=tmp_path)
    assert done.returncode == 2
    assert "incomplete" in done.stderr


@pytest.mark.skipif(
    not PACK_SCHEMA.exists(), reason="pack-manifest schema lands with PR #288"
)
def test_example_manifest_validates_against_the_pack_manifest_schema():
    from jsonschema import Draft202012Validator
    from referencing import Registry, Resource

    schemas = [
        json.loads(p.read_text("utf-8"))
        for p in (REPO / "schemas" / "v3").glob("*.schema.json")
    ]
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in schemas]
    )
    schema = json.loads(PACK_SCHEMA.read_text("utf-8"))
    manifest = tomllib.loads((EXAMPLE / "slopvac-pack.toml").read_text("utf-8"))
    errors = list(Draft202012Validator(schema, registry=registry).iter_errors(manifest))
    assert errors == []


def test_cli_gives_the_plugin_only_the_environment_it_is_told_to(tmp_path):
    dump = tmp_path / "env.txt"
    env = {**os.environ, "CI_DEPLOY_TOKEN": "hunter2", "OPTED_IN": "yes"}
    done = subprocess.run(
        [
            sys.executable,
            "-m",
            "slopvac_plugin.conformance",
            "--only",
            "P1",
            "--env",
            "OPTED_IN",
            "--",
            sys.executable,
            str(FIXTURES / "hostile_plugin.py"),
            "env-probe",
            str(dump),
        ],
        check=False,
        capture_output=True,
        text=True,
        cwd=tmp_path,
        env=env,
        timeout=120,
    )
    assert done.returncode == 1, done.stdout + done.stderr  # the probe is no plugin
    names = set(dump.read_text("utf-8").split())
    assert "OPTED_IN" in names
    assert "CI_DEPLOY_TOKEN" not in names


def fixer_doc(length: int) -> dict:
    return {"findings": [{"span": {"start": 0, "end": length}}]}


def edit(start: int, end: int, text: str = "") -> dict:
    return {"finding": 0, "span": {"start": start, "end": end}, "text": text}


def test_suite_accepts_a_thousand_edits_and_refuses_more_quickly():
    import time

    doc = fixer_doc(100_000)
    check_edits(doc, [edit(i, i) for i in range(1000)])
    started = time.monotonic()
    with pytest.raises(CaseFailure, match="1000"):
        check_edits(doc, [edit(i, i) for i in range(100_000)])
    assert time.monotonic() - started < 1


@pytest.mark.parametrize(
    "edits",
    [
        [edit(0, 10), edit(2, 2), edit(4, 5)],  # a wide edit and a far one
        [edit(2, 4), edit(3, 5)],
    ],
)
def test_suite_refuses_overlapping_edits(edits):
    with pytest.raises(CaseFailure, match="overlap"):
        check_edits(fixer_doc(10), edits)


def test_suite_accepts_touching_edits_and_insertions_at_one_point():
    check_edits(
        fixer_doc(10),
        [edit(2, 2), edit(2, 3), edit(3, 5), edit(5, 5), edit(7, 7), edit(7, 7)],
    )


def test_suite_refuses_inserted_bytes_over_the_limit():
    check_edits(fixer_doc(2), [edit(0, 2, "x" * 4104)])
    with pytest.raises(CaseFailure, match="bytes"):
        check_edits(fixer_doc(2), [edit(0, 2, "x" * 4105)])


def test_an_unknown_hook_kind_is_ignored_by_the_cases_that_call_hooks(quiet_cwd):
    # The plugin declares a tagger and a "hologram" hook. Section 4.2: ignore the latter.
    report = run_conformance(
        script("raw_plugin.py", "unknown-kind"), only=["P5", "P6"], timeout=5
    )
    assert statuses(report) == {"P5": "pass", "P6": "pass"}, report.to_text()


def _load_example():
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "fence_tagger_example", EXAMPLE / "plugin.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _context(cancelled: bool = False):
    import threading

    from slopvac_plugin import Context

    event = threading.Event()
    if cancelled:
        event.set()
    return Context(1, True, frozenset(), event)


def test_example_tags_a_document_with_many_fences_in_linear_time():
    """Round 3, finding 4: 60,000 non-ASCII fences took quadratic time to convert."""
    from slopvac_plugin import Document

    example = _load_example()
    doc = Document("d", "```\né\n```\n" * 60_000)
    out: list = []
    assert bounded(lambda: out.extend(example.tag_fences(doc, _context())), 5)
    assert len(out) == 60_000
    # The block "```\né\n```\n" is 3+1+2+1+3+1 = 11 bytes; the last one starts at 11 * 59999.
    assert out[-1].span == (11 * 59_999, 11 * 60_000)


def test_example_stops_when_the_request_is_cancelled():
    from slopvac_plugin import Document
    from slopvac_plugin.errors import RequestCancelled

    example = _load_example()
    with pytest.raises(RequestCancelled):
        example.tag_fences(
            Document("d", "```\nx\n```\n" * 10), _context(cancelled=True)
        )


def run_within(seconds, fn):
    """Result of ``fn()``; fails the test when it is still running after ``seconds``."""
    box = []
    assert bounded(lambda: box.append(fn()), seconds), (
        f"still running after {seconds} s"
    )
    return box[0]


@pytest.mark.parametrize("mode", ["mixed-features", "many-hooks"])
def test_an_oversized_capability_list_fails_p1_without_burning_cpu(mode, quiet_cwd):
    # Round 4, findings 3 and 4: 20,000+ entries made the schema's uniqueItems check,
    # and P5's membership test, quadratic. The reply is under 1 MiB, so framing admits it.
    report = run_within(
        15, lambda: run_conformance(script("raw_plugin.py", mode), timeout=10)
    )
    assert statuses(report)["P1"] == "fail", report.to_text()
    assert "1024" in next(r.detail for r in report.results if r.id == "P1")


def test_a_plugin_to_host_call_is_refused_before_its_params_are_validated(start):
    s = start(script("raw_plugin.py", "call-oversized"))
    handshake(s)

    def request():
        with pytest.raises(CaseFailure, match="plugin-to-host"):
            s.request(
                "tagger/tag",
                {"tagger": "main", "documents": [{"id": "d", "text": "t"}]},
            )

    run_within(15, request)


def test_the_suite_accepts_two_inserts_at_one_offset_from_an_sdk_fixer(quiet_cwd):
    # positions.md, Edit: zero-width edits at one offset apply in array order.
    report = run_conformance(
        script("double_insert_plugin.py"),
        inputs={},
        pass_env=SDK_ENV,
        only=["P15", "P18"],
    )
    assert statuses(report) == {"P15": "pass", "P18": "pass"}, report.to_text()


JAPANESE = "\u65e5\u672c\u8a9e" * 1334  # 4002 characters: 12006 bytes, 24012 escaped


def tagger_input(documents):
    return {"tagger": {"documents": documents}}


@pytest.mark.parametrize("case_id", ["P6", "P15"])
def test_batches_are_sized_by_the_escaped_bytes_the_session_sends(case_id, quiet_cwd):
    # Six documents of 12006 UTF-8 bytes. Counted as raw UTF-8, five fit in 65536
    # bytes, but the session sends them escaped (24012 bytes each), so two fit.
    # P6 sends them to a hook id the plugin does not declare.
    documents = [{"id": f"d{n}", "text": JAPANESE} for n in range(6)]
    report = run_conformance(
        script("raw_plugin.py", "small-limit"),
        inputs=tagger_input(documents),
        only=[case_id],
    )
    assert statuses(report) == {case_id: "pass"}, report.to_text()


def tag_body_size(request_id: int, hook_id: str, documents: list) -> int:
    """Bytes of the tagger/tag body the suite sends: compact JSON, default spacing."""
    message = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tagger/tag",
        "params": {"tagger": hook_id, "documents": documents},
    }
    return len(json.dumps(message).encode("utf-8"))


def test_batches_at_the_limit_are_sized_with_the_id_each_is_sent_with(quiet_cwd):
    # Every pair of documents makes a body of exactly 65536 bytes with a one-digit
    # id. Batches go out with ids 2, 3, ..., so the ninth carries id 10 and one more
    # byte: it must hold one document, or the plugin exits on the oversized frame.
    empty = [{"id": f"d{n:02}", "text": ""} for n in range(2)]
    room = 65536 - tag_body_size(2, "main", empty)
    even, odd = room // 2, room - room // 2
    documents = [
        {"id": f"d{n:02}", "text": "x" * (odd if n % 2 else even)} for n in range(18)
    ]
    assert tag_body_size(9, "main", documents[:2]) == 65536
    assert tag_body_size(10, "main", documents[:2]) == 65537
    report = run_conformance(
        script("raw_plugin.py", "small-limit"),
        inputs=tagger_input(documents),
        only=["P15"],
    )
    assert statuses(report) == {"P15": "pass"}, report.to_text()


def test_p7_sizes_each_planned_request_with_the_id_it_is_sent_with(quiet_cwd):
    # Nine taggers, one request each, ids 2 to 10. A valid and an invalid document
    # make exactly 65536 bytes with a one-digit id, so the request for t9 (id 10) is
    # one byte over and must stay unsent.
    bad = {"id": "bad", "text": "a\u0000b"}
    room = 65536 - tag_body_size(2, "t1", [{"id": "ok", "text": ""}, bad])
    valid = {"id": "ok", "text": "x" * room}
    assert tag_body_size(10, "t9", [valid, bad]) == 65537
    inputs = {"tagger": {"documents": [valid], "invalid_document": bad}}
    report = run_conformance(
        script("raw_plugin.py", "nine-taggers"), inputs=inputs, only=["P7"]
    )
    assert statuses(report) == {"P7": "pass"}, report.to_text()


def test_a_provider_request_over_the_limit_skips_p16_unsent(quiet_cwd):
    # The SDK plugin negotiates 16 MiB; one item of 17 MiB cannot fit.
    item = {"id": "item-1", "text": "x" * (17 * 1024 * 1024)}
    report = run_conformance(
        script("all_hooks_plugin.py"),
        inputs={"provider": {"items": [item]}},
        pass_env=SDK_ENV,
        only=["P16"],
    )
    assert statuses(report) == {"P16": "skip"}, report.to_text()


def test_a_document_over_the_limit_alone_skips_and_no_such_frame_is_written(
    quiet_cwd, monkeypatch
):
    sent = []
    original = Session.send_bytes

    def spy(self, data, **kwargs):
        sent.append(len(data.partition(b"\r\n\r\n")[2]))
        return original(self, data, **kwargs)

    monkeypatch.setattr(Session, "send_bytes", spy)
    huge = {"id": "huge", "text": JAPANESE * 3}  # 72036 UTF-8 bytes, over 65536 alone
    report = run_conformance(
        script("raw_plugin.py", "small-limit"),
        inputs=tagger_input([huge]),
        only=["P15"],
    )
    assert statuses(report) == {"P15": "skip"}, report.to_text()
    assert "huge" in report.results[0].detail
    assert max(sent) <= 65536  # the handshake only; the oversized request never left


def test_a_valid_split_into_singleton_batches_passes(quiet_cwd):
    report = run_conformance(script("raw_plugin.py", "batch1"), only=["P15"])
    assert statuses(report) == {"P15": "pass"}, report.to_text()


@pytest.mark.parametrize(
    "mode",
    [
        "batch1-swapped",  # each reply names the other document
        "batch1-both",  # the first reply answers both, the second answers none
        "batch1-malformed",  # the second reply has no results
    ],
)
def test_each_batch_reply_is_checked_against_its_own_request(mode, quiet_cwd):
    report = run_conformance(script("raw_plugin.py", mode), only=["P15"])
    assert statuses(report) == {"P15": "fail"}, report.to_text()


def test_a_case_that_outruns_its_budget_fails_instead_of_running_on(quiet_cwd):
    # 20 singleton batches at 0.3 s each would take 6 s.
    documents = [{"id": f"d{n}", "text": "x"} for n in range(20)]
    started = time.monotonic()
    report = run_conformance(
        script("raw_plugin.py", "batch1-slow"),
        inputs=tagger_input(documents),
        only=["P15"],
        case_budget=1.0,
    )
    assert statuses(report) == {"P15": "fail"}, report.to_text()
    assert "budget" in report.results[0].detail
    assert time.monotonic() - started < 4


def test_the_suite_budget_fails_the_cases_that_remain(quiet_cwd):
    documents = [{"id": f"d{n}", "text": "x"} for n in range(20)]
    report = run_conformance(
        script("raw_plugin.py", "batch1-slow"),
        inputs=tagger_input(documents),
        only=["P15", "P16"],
        suite_budget=1.0,
    )
    assert statuses(report) == {"P15": "fail", "P16": "fail"}, report.to_text()
    assert "suite budget" in report.results[1].detail


def test_cli_takes_the_budget_options(tmp_path):
    result = cli(
        "--case-budget",
        "30",
        "--suite-budget",
        "60",
        "--only",
        "P1",
        "--",
        *script("raw_plugin.py"),
        cwd=tmp_path,
    )
    assert result.returncode == 0, result.stdout


POSIX_ONLY = pytest.mark.skipif(os.name == "nt", reason="setsid is POSIX")


@POSIX_ONLY
@pytest.mark.parametrize("case_id", ["P13", "P14"])
def test_frames_after_exit_from_an_escaped_descendant_fail_at_once(
    case_id, escape_pidfile, quiet_cwd
):
    # A descendant in its own session holds standard output and prints a frame every
    # 0.4 s for 30 s after the plugin exits. No wait may restart on each frame.
    report = run_within(
        15,
        lambda: run_conformance(
            script("raw_plugin.py", "trickle-after-exit"),
            only=[case_id],
            pass_env=["ESCAPE_PIDFILE"],
            case_budget=10,
        ),
    )
    assert statuses(report) == {case_id: "fail"}, report.to_text()
    assert "printed" in report.results[0].detail


@POSIX_ONLY
@pytest.mark.parametrize(
    ("case_id", "inputs"),
    [
        ("P1", None),
        # A document over the 16 MiB limit: the case skips inside its session.
        ("P15", tagger_input([{"id": "big", "text": "x" * (17 * 1024 * 1024)}])),
    ],
)
def test_a_descendant_that_leaves_the_process_group_fails_the_case(
    case_id, inputs, escape_pidfile, quiet_cwd
):
    # The descendant holds standard output and stays silent, so the suite cannot end
    # it or see the end of output. The case reports it instead of passing or skipping.
    report = run_within(
        30,
        lambda: run_conformance(
            script("raw_plugin.py", "escape-silent"),
            inputs=inputs,
            only=[case_id],
            pass_env=["ESCAPE_PIDFILE"],
        ),
    )
    assert statuses(report) == {case_id: "fail"}, report.to_text()
    assert "process group" in report.results[0].detail


@pytest.mark.parametrize("outcome", ["returns", "skips"])
def test_a_case_that_ends_after_its_deadline_fails(outcome, monkeypatch, quiet_cwd):
    # The runner checks the clock after the case, whatever the case reported.
    def late(env):
        time.sleep(max(0.0, env.deadline - time.monotonic()) + 0.05)
        if outcome == "skips":
            raise CaseSkip("not applicable")

    monkeypatch.setitem(CASES, "P1", ("a case that ends late", late))
    report = run_conformance(script("raw_plugin.py"), only=["P1"], case_budget=1.0)
    assert statuses(report) == {"P1": "fail"}, report.to_text()
    assert "budget" in report.results[0].detail


def test_a_case_whose_cleanup_runs_past_its_budget_fails(quiet_cwd):
    # The initialize reply takes 1 s of the 1.5 s budget, and the plugin ignores end
    # of input, so closing the session would need 0.5 s more than the budget holds.
    report = run_conformance(
        script("raw_plugin.py", "ignore-eof"), only=["P1"], case_budget=1.5
    )
    assert statuses(report) == {"P1": "fail"}, report.to_text()
    assert "budget" in report.results[0].detail
