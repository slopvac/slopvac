from __future__ import annotations

import importlib.util
import json
import re
import signal
import stat
import sys
from pathlib import Path

import pytest
import yaml
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

pytest_plugins = ["real_binary"]  # the real-binary fixtures

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "action_v3.py"
ACTION = ROOT / "action-v3" / "action.yml"
SCHEMAS = ROOT / "schemas" / "v3"
SPEC = importlib.util.spec_from_file_location("action_v3", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
action = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = action
SPEC.loader.exec_module(action)

PR = {
    "EVENT_NAME": "pull_request",
    "HEAD_REPO": "o/r",
    "BASE_REPO": "o/r",
    "ACTOR": "dev",
}
FORK = {**PR, "HEAD_REPO": "fork/r"}
PUSH = {"EVENT_NAME": "push"}


@pytest.mark.parametrize(
    ("env", "expected"),
    [
        (PR, ("github-pr-review", True)),
        (FORK, ("github-pr-annotations", False)),
        ({**PR, "ACTOR": "dependabot[bot]"}, ("github-pr-annotations", False)),
        ({**PR, "HEAD_REPO": ""}, ("github-pr-annotations", False)),
        ({**PR, "REPORTER": "annotations"}, ("github-pr-annotations", False)),
        ({**FORK, "REPORTER": "review"}, ("github-pr-annotations", False)),
        ({**PR, "REPORTER": "none"}, ("none", False)),
        (PUSH, ("github-check", False)),
    ],
)
def test_reporter_choice(env, expected):
    assert action.choose_reporter(env) == expected


def test_unknown_reporter_is_an_error():
    with pytest.raises(action.ActionError, match="reporter must be"):
        action.choose_reporter({**PR, "REPORTER": "slack"})


FORMATS = ["json", "rdjsonl", "sarif", "codeclimate"]


# A process double for what the real binary cannot be told to do: die from a
# signal, exit 70, fail `plugins sync` (not wired in the binary yet) and log
# the exact argv it received. It writes no report content; every test of report
# content runs the real binary (see real_binary.py).
PROCESS_DOUBLE = """#!/usr/bin/env python3
import json, os, signal, sys

args = sys.argv[1:]
if os.environ.get("FAKE_LOG"):
    with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as handle:
        handle.write(json.dumps(args) + "\\n")
if os.environ.get("FAKE_STDERR"):
    sys.stderr.buffer.write(os.environ["FAKE_STDERR"].encode("utf-8", "surrogateescape"))
if "plugins" in args and "sync" in args:
    sys.exit(int(os.environ.get("FAKE_EXIT_SYNC", "0")))
fmt = args[args.index("--format") + 1]
with open(args[args.index("--out") + 1], "w", encoding="utf-8") as handle:
    handle.write("{}\\n")
if os.environ.get("FAKE_SIGNAL_" + fmt.upper()):
    os.kill(os.getpid(), signal.Signals(int(os.environ["FAKE_SIGNAL_" + fmt.upper()])))
sys.exit(int(os.environ.get("FAKE_EXIT_" + fmt.upper(), "1")))
"""


@pytest.fixture
def binary(tmp_path):
    """The process double, with every format exiting 0 unless a test says otherwise."""
    target = tmp_path / "slopvac"
    target.write_text(PROCESS_DOUBLE, encoding="utf-8")
    target.chmod(target.stat().st_mode | stat.S_IXUSR)
    return target


@pytest.fixture
def run(tmp_path, binary):
    paths = tmp_path / "paths"
    paths.write_bytes(b"docs/a.md\0docs/b c.md\0--odd.md\0")
    github_output = tmp_path / "out"
    base = {
        "BINARY": str(binary),
        "PATH_LIST": str(paths),
        "RUNNER_TEMP": str(tmp_path),
        "GITHUB_WORKSPACE": str(tmp_path),
        "GITHUB_OUTPUT": str(github_output),
        "FAKE_LOG": str(tmp_path / "log"),
        "SARIF_FILE": str(tmp_path / "x.sarif"),
        "CODECLIMATE_FILE": str(tmp_path / "x.json"),
        **{f"FAKE_EXIT_{fmt.upper()}": "0" for fmt in FORMATS},
    }

    def go(**extra):
        (tmp_path / "log").unlink(missing_ok=True)
        github_output.unlink(missing_ok=True)
        code = action.lint({**base, **extra})
        log = [
            json.loads(line)
            for line in (tmp_path / "log").read_text(encoding="utf-8").splitlines()
        ]
        outputs = dict(
            line.split("=", 1)
            for line in github_output.read_text(encoding="utf-8").splitlines()
        )
        return code, log, outputs

    return go


def _format(argv):
    return argv[argv.index("--format") + 1]


def test_each_report_is_rendered_and_arguments_stay_whole(run, tmp_path):
    _, log, outputs = run(FAKE_EXIT_JSON="1")
    assert [_format(argv) for argv in log] == FORMATS
    for argv in log:
        assert argv[: argv.index("--format")] == ["--offline", "lint"]
        assert argv[argv.index("--") :] == [
            "--",
            "docs/a.md",
            "docs/b c.md",
            "--odd.md",
        ]
    assert outputs["sarif-file"] == str(tmp_path / "x.sarif")


@pytest.mark.parametrize(
    "env", [{}, {"OFFLINE": "false"}], ids=["default", "env-override"]
)
def test_every_lint_invocation_is_offline(run, env):
    _, log, _ = run(**env)
    assert len(log) == len(FORMATS)
    assert all(argv[0] == "--offline" for argv in log)


def test_default_report_destinations_land_in_the_workspace_not_the_runner_temp(
    tmp_path, binary
):
    workspace, temp = tmp_path / "ws", tmp_path / "tmp"
    workspace.mkdir()
    temp.mkdir()
    paths = tmp_path / "paths"
    paths.write_bytes(b"a.md\0")
    github_output = tmp_path / "out"
    env = {
        "BINARY": str(binary),
        "PATH_LIST": str(paths),
        "RUNNER_TEMP": str(temp),
        "GITHUB_WORKSPACE": str(workspace),
        "GITHUB_OUTPUT": str(github_output),
        "FAKE_LOG": str(tmp_path / "log"),
        **{f"FAKE_EXIT_{fmt.upper()}": "0" for fmt in FORMATS},
    }
    assert action.lint(env) == 0
    assert (workspace / "slopvac.rdjsonl").is_file()
    assert (workspace / "slopvac.sarif").is_file()
    assert (workspace / "slopvac-codeclimate.json").is_file()
    assert (temp / "slopvac-report.json").is_file()
    assert not (temp / "slopvac.rdjsonl").exists()


def test_rdjsonl_is_written_to_a_nested_workspace_path(real_run):
    target = real_run.workspace / "reports" / "sub" / "slopvac.rdjsonl"
    code, outputs = real_run(
        "slop.md", RDJSONL_FILE="reports/sub/slopvac.rdjsonl", FAIL_ON_FINDINGS="false"
    )
    assert code == 0
    assert outputs["rdjsonl-file"] == str(target)
    first = json.loads(target.read_text(encoding="utf-8").splitlines()[0])
    assert first["source"] == {"name": "slopvac"}


def test_rdjsonl_survives_a_findings_failure_at_the_requested_path(run, tmp_path):
    target = tmp_path / "findings" / "findings.rdjsonl"
    code, _, _ = run(RDJSONL_FILE=str(target), FAKE_EXIT_JSON="1")
    assert code == 1
    assert target.is_file()


@pytest.mark.parametrize("variable", ["RDJSONL_FILE", "SARIF_FILE", "CODECLIMATE_FILE"])
@pytest.mark.parametrize("escape", ["../outside.out", "/etc/slopvac.out"])
def test_report_destinations_outside_the_workspace_are_refused(
    tmp_path, binary, variable, escape
):
    workspace = tmp_path / "ws"
    workspace.mkdir()
    paths = tmp_path / "paths"
    paths.write_bytes(b"a.md\0")
    env = {
        "BINARY": str(binary),
        "PATH_LIST": str(paths),
        "RUNNER_TEMP": str(tmp_path),
        "GITHUB_WORKSPACE": str(workspace),
        "FAKE_LOG": str(tmp_path / "log"),
        variable: escape,
    }
    with pytest.raises(action.ActionError, match="outside the workspace"):
        action.lint(env)
    assert not (tmp_path / "log").exists()
    assert not (tmp_path / "outside.out").exists()


@pytest.mark.parametrize("where", ["file", "directory"])
def test_report_destinations_through_a_symlink_are_refused(tmp_path, where):
    workspace = tmp_path / "ws"
    (workspace / "real").mkdir(parents=True)
    victim = tmp_path / "victim"
    victim.write_text("keep", encoding="utf-8")
    (workspace / "link-file").symlink_to(victim)
    (workspace / "link-dir").symlink_to(workspace / "real")
    raw = "link-file" if where == "file" else "link-dir/slopvac.rdjsonl"
    with pytest.raises(action.ActionError, match="symlink"):
        action.workspace_path(raw, {"GITHUB_WORKSPACE": str(workspace)})
    assert victim.read_text(encoding="utf-8") == "keep"


def test_a_symlinked_workspace_is_refused(tmp_path):
    (tmp_path / "real").mkdir()
    (tmp_path / "ws").symlink_to(tmp_path / "real")
    with pytest.raises(action.ActionError, match="symlink"):
        action.workspace_path("a.rdjsonl", {"GITHUB_WORKSPACE": str(tmp_path / "ws")})


def test_a_relative_destination_is_taken_from_the_workspace(tmp_path):
    resolved = action.workspace_path(
        "findings/../out/slopvac.rdjsonl", {"GITHUB_WORKSPACE": str(tmp_path)}
    )
    assert resolved == tmp_path.resolve() / "out" / "slopvac.rdjsonl"


def test_the_lint_step_receives_the_rdjsonl_input():
    action_yml = yaml.safe_load(ACTION.read_text(encoding="utf-8"))
    assert action_yml["inputs"]["rdjsonl-file"]["default"]
    assert _step("Lint")["env"]["RDJSONL_FILE"] == "${{ inputs.rdjsonl-file }}"


def _fork_template_steps():
    doc = (ROOT / "docs" / "ci" / "github-action.md").read_text(encoding="utf-8")
    blocks = re.findall(r"```yaml\n(.*?)```", doc, re.DOTALL)
    template = next(b for b in blocks if "slopvac-findings" in b)
    workflow = yaml.safe_load(template.replace("# .github", "#"))
    return workflow["jobs"]["lint"]["steps"]


def test_fork_template_uploads_the_report_path_it_asks_the_action_to_write():
    steps = _fork_template_steps()
    written = next(s for s in steps if s.get("id") == "slopvac")["with"]["rdjsonl-file"]
    upload = next(s for s in steps if "upload-artifact" in s.get("uses", ""))
    assert upload["if"] == f"${{{{ !cancelled() && hashFiles('{written}') != '' }}}}"
    assert Path(written).parent.as_posix() == upload["with"]["path"]


def test_reports_can_be_switched_off(run):
    _, log, _ = run(SARIF="false", CODECLIMATE="false")
    assert [_format(argv) for argv in log] == ["json", "rdjsonl"]


def test_threshold_flags_reach_every_run(run):
    _, log, _ = run(MIN_SCORE="80", MAX_PER_100="2.5", CONFIG="ci/slopvac.toml")
    expected = ["--config", "ci/slopvac.toml", "--min-score", "80"]
    expected += ["--max-per-100-words", "2.5"]
    for argv in log:
        start = argv.index("--config")
        assert argv[start : start + 6] == expected


@pytest.mark.parametrize(
    ("env", "code"),
    [
        ({"FAKE_EXIT_JSON": "1"}, 1),
        ({"FAKE_EXIT_JSON": "1", "FAIL_ON_FINDINGS": "false"}, 0),
        ({"FAKE_EXIT_RDJSONL": "1"}, 0),
        ({}, 0),
    ],
)
def test_findings_exit_is_the_only_softened_verdict(run, env, code):
    assert run(**env)[0] == code


CRASHES = [
    pytest.param({"EXIT": "2"}, id="exit-2"),
    pytest.param({"EXIT": "3"}, id="exit-3"),
    pytest.param({"EXIT": "70"}, id="exit-70"),
    pytest.param({"SIGNAL": str(int(signal.SIGKILL))}, id="killed-by-signal"),
]


@pytest.mark.parametrize("fail_on_findings", ["true", "false"])
@pytest.mark.parametrize("crash", CRASHES)
@pytest.mark.parametrize("fmt", FORMATS)
def test_an_unexpected_end_fails_from_every_invocation(
    run, fmt, crash, fail_on_findings
):
    env = {f"FAKE_{kind}_{fmt.upper()}": value for kind, value in crash.items()}
    code, log, _ = run(FAIL_ON_FINDINGS=fail_on_findings, **env)
    assert code == 2
    assert [_format(argv) for argv in log] == FORMATS  # the other reports still ran


def test_no_targets_passes_without_running_the_binary(tmp_path):
    paths = tmp_path / "paths"
    paths.write_bytes(b"")
    out = tmp_path / "out"
    code = action.lint(
        {"PATH_LIST": str(paths), "GITHUB_OUTPUT": str(out), "BINARY": "/nonexistent"}
    )
    assert (code, out.read_text(encoding="utf-8")) == (0, "passed=true\n")


# ---- binary stderr never reaches the job log as a workflow command -----------

# A target path with a newline reaches the binary's stderr verbatim. Expected
# values are worked out by hand from GitHub's workflow-command rules: `::`
# becomes %3A%3A, `%` becomes %25, `##[` is neutralised, a bell is shown as
# \u{7}, and each line stays a line.
HOSTILE_STDERR = (
    "cannot read a.md\n::error::x %0A\n::set-output name=p::1\n##[group]g\a\n"
)
ESCAPED_STDERR = (
    "cannot read a.md\n"
    "%3A%3Aerror%3A%3Ax %250A\n"
    "%3A%3Aset-output name=p%3A%3A1\n"
    "\\u{23}#[group]g\\u{7}\n"
)


def _assert_no_workflow_command(text):
    assert not [ln for ln in text.splitlines() if ln.lstrip().startswith("::")]


def test_lint_stderr_is_escaped_before_it_reaches_the_job_log(run, capfd):
    run(FAKE_STDERR=HOSTILE_STDERR)
    err = capfd.readouterr().err
    _assert_no_workflow_command(err)
    # one copy per report format: the four runs each emit the same text
    assert err == ESCAPED_STDERR * len(FORMATS)


def test_sync_stderr_is_escaped_before_it_reaches_the_job_log(sync, capfd):
    sync(locked=True, FAKE_STDERR=HOSTILE_STDERR)
    err = capfd.readouterr().err
    _assert_no_workflow_command(err)
    assert err == ESCAPED_STDERR


def test_stderr_that_is_not_utf8_is_shown_not_dropped(sync, capfd):
    sync(locked=True, FAKE_STDERR="bad \udcff byte\n")
    assert capfd.readouterr().err == "bad \ufffd byte\n"


# ---- plugins sync ------------------------------------------------------------


@pytest.fixture
def sync(tmp_path, binary):
    lock = tmp_path / ".slopvac" / "packs.lock"

    def go(*, locked: bool, **extra):
        if locked:
            lock.parent.mkdir(exist_ok=True)
            lock.write_text("schema_version = 1\n", encoding="utf-8")
        log = tmp_path / "log"
        log.unlink(missing_ok=True)
        env = {"BINARY": str(binary), "FAKE_LOG": str(log), "LOCKFILE": str(lock)}
        code = action.sync({**env, **extra})
        calls = (
            [json.loads(x) for x in log.read_text(encoding="utf-8").splitlines()]
            if log.exists()
            else []
        )
        return code, calls

    return go


def test_a_lockfile_syncs_packs(sync, binary):
    assert sync(locked=True) == (0, [["plugins", "sync"]])


def test_no_lockfile_means_no_sync(sync):
    assert sync(locked=False) == (0, [])


@pytest.mark.parametrize("status", ["1", "2", "70"])
def test_a_failed_sync_fails_the_action(sync, status):
    assert sync(locked=True, FAKE_EXIT_SYNC=status)[0] == 2


def _steps():
    return yaml.safe_load(ACTION.read_text(encoding="utf-8"))["runs"]["steps"]


def test_sync_is_a_named_step_before_the_lint_step():
    names = [step["name"] for step in _steps()]
    assert names.index("Sync packs") < names.index("Lint")
    sync_step = _steps()[names.index("Sync packs")]
    assert sync_step["run"].endswith('action_v3.py" sync')
    assert "continue-on-error" not in sync_step


# ---- reviewdog ---------------------------------------------------------------

RD_FAKE = """#!/usr/bin/env python3
import json, os, sys
reporter = [a for a in sys.argv if a.startswith("-reporter=")][0].split("=", 1)[1]
with open(os.environ["RD_LOG"], "a") as log:
    log.write(json.dumps({"argv": sys.argv[1:], "stdin": sys.stdin.read()}) + "\\n")
sys.exit(1 if reporter in os.environ.get("RD_FAIL", "").split(",") else 0)
"""


@pytest.fixture
def report(tmp_path):
    reviewdog = tmp_path / "reviewdog"
    reviewdog.write_text(RD_FAKE, encoding="utf-8")
    reviewdog.chmod(reviewdog.stat().st_mode | stat.S_IXUSR)
    rdjsonl = tmp_path / "slopvac.rdjsonl"
    rdjsonl.write_text('{"message": "m"}\n', encoding="utf-8")
    log = tmp_path / "rd.log"

    def go(env, **extra):
        log.unlink(missing_ok=True)
        base = {
            "REVIEWDOG": str(reviewdog),
            "RDJSONL": str(rdjsonl),
            "RD_LOG": str(log),
        }
        code = action.report({**env, **base, **extra})
        calls = (
            [json.loads(x) for x in log.read_text(encoding="utf-8").splitlines()]
            if log.exists()
            else []
        )
        return (
            code,
            [
                next(a for a in c["argv"] if a.startswith("-reporter="))[10:]
                for c in calls
            ],
            calls,
        )

    return go


@pytest.mark.parametrize(
    ("env", "fail", "attempts"),
    [
        pytest.param(PR, "", ["github-pr-review"], id="writable-same-repo"),
        pytest.param(
            PR,
            "github-pr-review",
            ["github-pr-review", "github-pr-annotations"],
            id="read-only-same-repo",
        ),
        pytest.param(FORK, "", ["github-pr-annotations"], id="fork"),
        pytest.param(
            FORK, "github-pr-annotations", ["github-pr-annotations"], id="fork-failing"
        ),
        pytest.param(PUSH, "", ["github-check"], id="push"),
        pytest.param(
            PR,
            "github-pr-review,github-pr-annotations",
            ["github-pr-review", "github-pr-annotations"],
            id="both-fail",
        ),
        pytest.param({**PR, "REPORTER": "none"}, "", [], id="none"),
    ],
)
def test_reporter_attempts_never_gate(report, env, fail, attempts):
    code, seen, _ = report(env, RD_FAIL=fail)
    assert (code, seen) == (0, attempts)


def test_reviewdog_reads_the_rdjsonl_report(report):
    _, _, calls = report(PR)
    assert calls[0]["argv"][0] == "-f=rdjsonl"
    assert calls[0]["stdin"] == '{"message": "m"}\n'


def test_a_missing_reviewdog_only_warns(report, capsys):
    code, _, _ = report(PR, REVIEWDOG="/nonexistent/reviewdog")
    assert code == 0
    assert "::warning" in capsys.readouterr().out


def test_the_fallback_is_logged(report, capsys):
    report(PR, RD_FAIL="github-pr-review")
    assert "falling back to github-pr-annotations" in capsys.readouterr().out


def test_a_missing_report_file_only_warns(report):
    code, seen, _ = report(PR, RDJSONL="/nonexistent.rdjsonl")
    assert (code, seen) == (0, [])


def test_reviewdog_install_failure_does_not_fail_the_action():
    install = next(s for s in _steps() if s["name"] == "Install reviewdog")
    assert install["continue-on-error"] is True


# ---- the real binary: reports are contract-valid and reach their consumers ----


@pytest.fixture
def real_run(slop_project, slopvac_binary, tmp_path, monkeypatch):
    """Run ``action.lint`` with the real binary on documents of ``slop_project``."""
    monkeypatch.chdir(slop_project)
    workspace = tmp_path / "ws"
    workspace.mkdir()

    def go(*documents, **extra):
        paths = tmp_path / "paths"
        paths.write_bytes("".join(f"{d}\0" for d in documents).encode())
        github_output = tmp_path / "out"
        github_output.unlink(missing_ok=True)
        env = {
            "BINARY": str(slopvac_binary),
            "PATH_LIST": str(paths),
            "RUNNER_TEMP": str(tmp_path),
            "GITHUB_WORKSPACE": str(workspace),
            "GITHUB_OUTPUT": str(github_output),
            **extra,
        }
        code = action.lint(env)
        outputs = dict(
            line.split("=", 1)
            for line in github_output.read_text(encoding="utf-8").splitlines()
        )
        return code, outputs

    go.workspace = workspace
    return go


@pytest.fixture
def reports(real_run):
    code, outputs = real_run("slop.md")
    assert code == 1
    return outputs, {
        fmt: Path(outputs[f"{fmt}-file"]).read_text(encoding="utf-8") for fmt in FORMATS
    }


def test_a_sloppy_document_fails_the_gate_with_the_json_summary(reports):
    outputs, texts = reports
    summary = json.loads(texts["json"])["summary"]
    assert summary["passed"] is False
    assert summary["findings"] == int(outputs["findings"]) > 0
    assert outputs["passed"] == "false"


@pytest.mark.parametrize("document", ["slop.md", "clean.md"])
def test_the_action_forwards_the_score_of_the_json_summary(real_run, document):
    _, outputs = real_run(document, FAIL_ON_FINDINGS="false")
    report = json.loads(Path(outputs["json-file"]).read_text(encoding="utf-8"))
    assert float(outputs["score"]) == report["summary"]["score"]


def test_a_clean_document_passes_the_gate(real_run):
    code, outputs = real_run("clean.md")
    assert (code, outputs["passed"]) == (0, "true")


def test_every_format_reports_the_same_number_of_findings(reports):
    outputs, texts = reports
    (run_,) = json.loads(texts["sarif"])["runs"]
    counts = {
        "json": int(outputs["findings"]),
        "rdjsonl": len(texts["rdjsonl"].splitlines()),
        "sarif": len(run_["results"]),
        "codeclimate": len(json.loads(texts["codeclimate"])),
    }
    assert set(counts.values()) == {counts["json"]}


def test_the_json_report_matches_the_lint_report_schema(reports):
    schemas = [json.loads(p.read_text("utf-8")) for p in SCHEMAS.glob("*.schema.json")]
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in schemas]
    )
    schema = json.loads((SCHEMAS / "lint-report.schema.json").read_text("utf-8"))
    errors = list(
        Draft202012Validator(schema, registry=registry).iter_errors(
            json.loads(reports[1]["json"])
        )
    )
    assert errors == []


# "This groundbreaking solution ...": the promotional phrase spans columns 6-29
# of line 1 of the sloppy fixture, counted by hand from the text.
PROMOTION_RANGE = {"start": {"line": 1, "column": 6}, "end": {"line": 1, "column": 29}}


def test_the_rdjsonl_report_is_what_reviewdog_reads(reports):
    diagnostics = [json.loads(line) for line in reports[1]["rdjsonl"].splitlines()]
    assert diagnostics
    for diagnostic in diagnostics:
        assert diagnostic["source"] == {"name": "slopvac"}
        assert diagnostic["severity"] in {"ERROR", "WARNING", "INFO"}
        assert diagnostic["location"]["path"] == "slop.md"
        assert diagnostic["message"] and diagnostic["code"]["value"]
    promoted = next(
        d for d in diagnostics if d["code"]["value"].startswith("core:prose-promotion.")
    )
    assert promoted["location"]["range"] == PROMOTION_RANGE


def test_the_sarif_report_is_what_code_scanning_reads(reports):
    sarif = json.loads(reports[1]["sarif"])
    assert sarif["version"] == "2.1.0"
    (run_,) = sarif["runs"]
    assert run_["tool"]["driver"]["name"] == "slopvac"
    promoted = next(
        r for r in run_["results"] if r["ruleId"].startswith("core:prose-promotion.")
    )
    assert promoted["locations"][0]["physicalLocation"] == {
        "artifactLocation": {"uri": "slop.md"},
        "region": {"startLine": 1, "startColumn": 6, "endLine": 1, "endColumn": 29},
    }


def test_the_codeclimate_report_is_an_issue_array(reports):
    issues = json.loads(reports[1]["codeclimate"])
    assert issues
    for issue in issues:
        assert issue["type"] == "issue"
        assert issue["location"]["path"] == "slop.md"
        assert re.fullmatch(r"[0-9a-f]{16}", issue["fingerprint"])


@pytest.mark.parametrize(
    ("env", "reporters"),
    [
        pytest.param(PR, ["github-pr-review"], id="same-repo"),
        pytest.param(FORK, ["github-pr-annotations"], id="fork"),
    ],
)
def test_reviewdog_receives_the_real_rdjsonl_on_same_repo_and_fork_prs(
    report, reports, env, reporters
):
    rdjsonl = reports[1]["rdjsonl"]
    _, seen, calls = report(env, RDJSONL=reports[0]["rdjsonl-file"])
    assert seen == reporters
    assert calls[0]["stdin"] == rdjsonl


def _step(name):
    return next(s for s in _steps() if s["name"] == name)


def test_sarif_upload_reads_the_lint_output_with_the_category():
    upload = _step("Upload SARIF to code scanning")
    assert upload["with"] == {
        "sarif_file": "${{ steps.lint.outputs.sarif-file }}",
        "category": "${{ inputs.sarif-category }}",
    }


def test_codeclimate_artifact_reads_the_lint_output():
    upload = _step("Upload the Code Climate report")
    assert upload["with"]["path"] == "${{ steps.lint.outputs.codeclimate-file }}"


def test_reviewdog_reads_the_lint_rdjsonl_output():
    assert (
        _step("Report with reviewdog")["env"]["RDJSONL"]
        == "${{ steps.lint.outputs.rdjsonl-file }}"
    )


def test_expressions_reach_scripts_only_through_env_and_actions_are_pinned():
    for step in _steps():
        assert "${{" not in step.get("run", ""), step["name"]
        if "uses" in step and not step["uses"].startswith("./"):
            assert re.search(r"@[0-9a-f]{40}$", step["uses"]), step["uses"]
