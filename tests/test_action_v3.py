from __future__ import annotations

import importlib.util
import stat
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "action_v3.py"
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


@pytest.mark.parametrize(
    ("env", "expected"),
    [
        (PR, ("github-pr-review", True)),
        ({**PR, "HEAD_REPO": "fork/r"}, ("github-pr-annotations", False)),
        ({**PR, "ACTOR": "dependabot[bot]"}, ("github-pr-annotations", False)),
        ({**PR, "HEAD_REPO": ""}, ("github-pr-annotations", False)),
        ({**PR, "REPORTER": "annotations"}, ("github-pr-annotations", False)),
        (
            {**PR, "REPORTER": "review", "HEAD_REPO": "fork/r"},
            ("github-pr-annotations", False),
        ),
        ({**PR, "REPORTER": "none"}, ("none", False)),
        ({"EVENT_NAME": "push"}, ("github-check", False)),
    ],
)
def test_reporter_choice(env, expected):
    assert action.choose_reporter(env) == expected


def test_unknown_reporter_is_an_error():
    with pytest.raises(action.ActionError, match="reporter must be"):
        action.choose_reporter({**PR, "REPORTER": "slack"})


# STUB(slopvac-8l4.13.12): this script stands in for the 3.0 binary, whose lint
# command and renderers are not on main. It writes canned reports.
FAKE = """#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
fmt = args[args.index("--format") + 1]
out = args[args.index("--out") + 1]
open(os.environ["FAKE_LOG"], "a").write(" ".join(args) + "\\n")
summary = {"score": 91.5, "findings": 3, "errors": 1, "warnings": 2, "suggestions": 0, "passed": False}
body = json.dumps({"summary": summary}) if fmt == "json" else fmt
open(out, "w").write(body)
sys.exit(int(os.environ.get("FAKE_EXIT_" + fmt.upper(), "0")))
"""


@pytest.fixture
def run(tmp_path, monkeypatch):
    binary = tmp_path / "slopvac"
    binary.write_text(FAKE, encoding="utf-8")
    binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
    paths = tmp_path / "paths"
    paths.write_bytes(b"docs/a.md\0docs/b c.md\0")
    github_output = tmp_path / "out"
    base = {
        "BINARY": str(binary),
        "PATH_LIST": str(paths),
        "RUNNER_TEMP": str(tmp_path),
        "GITHUB_OUTPUT": str(github_output),
        "FAKE_LOG": str(tmp_path / "log"),
        "SARIF_FILE": str(tmp_path / "x.sarif"),
        "CODECLIMATE_FILE": str(tmp_path / "x.json"),
    }

    def go(**extra):
        (tmp_path / "log").unlink(missing_ok=True)
        github_output.unlink(missing_ok=True)
        code = action.lint({**base, **extra})
        log = (tmp_path / "log").read_text(encoding="utf-8").splitlines()
        outputs = dict(
            line.split("=", 1)
            for line in github_output.read_text(encoding="utf-8").splitlines()
        )
        return code, log, outputs

    return go


def test_each_report_is_rendered_and_paths_stay_whole(run, tmp_path):
    code, log, outputs = run(FAKE_EXIT_JSON="1")
    formats = [line.split("--format ")[1].split()[0] for line in log]
    assert formats == ["json", "rdjsonl", "sarif", "codeclimate"]
    assert all(line.endswith("docs/a.md docs/b c.md") for line in log)
    assert (tmp_path / "x.sarif").read_text(encoding="utf-8") == "sarif"
    assert (code, outputs["findings"], outputs["passed"], outputs["score"]) == (
        1,
        "3",
        "false",
        "91.5",
    )


def test_the_binary_runs_offline_by_default_and_online_on_request(run):
    _, log, _ = run()
    assert all(line.startswith("--offline lint ") for line in log)
    _, log, _ = run(OFFLINE="false")
    assert all(line.startswith("lint ") for line in log)


def test_reports_can_be_switched_off(run):
    _, log, _ = run(SARIF="false", CODECLIMATE="false")
    assert [line.split("--format ")[1].split()[0] for line in log] == [
        "json",
        "rdjsonl",
    ]


@pytest.mark.parametrize(
    ("env", "code"),
    [
        ({"FAKE_EXIT_JSON": "1"}, 1),
        ({"FAKE_EXIT_JSON": "1", "FAIL_ON_FINDINGS": "false"}, 0),
        ({"FAKE_EXIT_JSON": "2", "FAIL_ON_FINDINGS": "false"}, 2),
        ({"FAKE_EXIT_SARIF": "2", "FAIL_ON_FINDINGS": "false"}, 2),
        ({}, 0),
    ],
)
def test_the_binary_exit_code_is_the_gate(run, env, code):
    assert run(**env)[0] == code


def test_threshold_flags_reach_the_gate_run(run):
    _, log, _ = run(MIN_SCORE="80", MAX_PER_100="2.5", CONFIG="ci/slopvac.toml")
    assert "--config ci/slopvac.toml --min-score 80 --max-per-100-words 2.5" in log[0]


def test_no_targets_passes_without_running_the_binary(tmp_path):
    paths = tmp_path / "paths"
    paths.write_bytes(b"")
    out = tmp_path / "out"
    code = action.lint(
        {"PATH_LIST": str(paths), "GITHUB_OUTPUT": str(out), "BINARY": "/nonexistent"}
    )
    assert (code, out.read_text(encoding="utf-8")) == (0, "passed=true\n")
