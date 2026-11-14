from __future__ import annotations

import json
import os
from pathlib import Path
from subprocess import CompletedProcess
from unittest.mock import patch

import pytest

from scripts import action_entry


def test_parse_input_paths_preserves_spaces() -> None:
    assert action_entry.parse_input_paths('README.md "docs/release notes.md"') == [
        "README.md",
        "docs/release notes.md",
    ]


def test_lint_writes_json_outputs_and_keeps_path_argument(tmp_path: Path) -> None:
    path_list = tmp_path / "targets"
    path_list.write_bytes(b"docs/release notes.md\0")
    github_output = tmp_path / "output"
    summary_path = tmp_path / "summary"
    report = {
        "summary": {
            "score": 91.5,
            "findings": 1,
            "errors": 0,
            "warnings": 1,
            "suggestions": 0,
            "documents": 1,
            "words": 100,
            "per_100_words": 1.0,
            "passed": True,
            "categories": [
                {
                    "category": "style",
                    "findings": 1,
                    "errors": 0,
                    "warnings": 1,
                    "per_100_words": 1.0,
                    "score": 91.5,
                }
            ],
        },
        "documents": [],
    }
    environment = {
        "PATH_LIST": str(path_list),
        "GITHUB_OUTPUT": str(github_output),
        "GITHUB_STEP_SUMMARY": str(summary_path),
        "RUNNER_TEMP": str(tmp_path),
        "SOURCE": "./package with spaces",
        "VALE": "false",
        "ANNOTATE": "false",
        "SARIF": "false",
        "FAIL_ON_FINDINGS": "true",
    }

    def fake_run(command: list[str], **kwargs: object) -> CompletedProcess[str]:
        assert "docs/release notes.md" in command
        assert "./package with spaces" in command
        output = kwargs["stdout"]
        assert hasattr(output, "write")
        output.write(json.dumps(report))
        return CompletedProcess(command, 1, stderr="")

    with (
        patch.dict(os.environ, environment, clear=False),
        patch("scripts.action_entry.subprocess.run", side_effect=fake_run),
    ):
        status = action_entry.lint()

    assert status == 1
    output = github_output.read_text()
    assert "score=91.5" in output
    assert "findings=1" in output
    assert "passed=true" in output
    assert "json=" in output
    assert "sarif=" in output
    assert "score 91.5/100" in summary_path.read_text()


def test_changed_only_without_pull_request_base_fails_closed(
    tmp_path, monkeypatch, capsys
):
    monkeypatch.setenv("CHANGED_ONLY", "true")
    monkeypatch.setenv("INPUT_PATHS", ".")
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))
    monkeypatch.delenv("BASE_SHA", raising=False)
    assert action_entry.resolve_targets() == 1
    assert "requires" in capsys.readouterr().out
    assert not (tmp_path / "slopvac-paths").exists()


def test_changed_only_resolves_exact_base_and_prose_paths(tmp_path, monkeypatch):
    monkeypatch.setenv("CHANGED_ONLY", "true")
    monkeypatch.setenv("INPUT_PATHS", "ignored.md")
    monkeypatch.setenv("BASE_SHA", "abc123")
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))
    calls: list[list[str]] = []

    def fake_run(command, **kwargs):
        calls.append(command)
        if command[:3] == ["git", "cat-file", "-e"]:
            return CompletedProcess(command, 0, stdout="", stderr="")
        return CompletedProcess(
            command, 0, stdout=b"docs/release notes.md\0", stderr=b""
        )

    with patch("scripts.action_entry.subprocess.run", side_effect=fake_run):
        assert action_entry.resolve_targets() == 0

    assert calls[0] == ["git", "cat-file", "-e", "abc123^{commit}"]
    assert "abc123...HEAD" in calls[1]
    assert (
        calls[1][-7:]
        == ["*.md", "*.mdx", "*.markdown", "*.txt", "*.rst", "*.html", "--"]
        or "--" in calls[1]
    )
    assert action_entry._read_paths(tmp_path / "slopvac-paths") == [
        "docs/release notes.md"
    ]


def test_command_passes_exact_diff_base_and_path_argv(monkeypatch):
    monkeypatch.setenv("CHANGED_ONLY", "true")
    monkeypatch.setenv("BASE_SHA", "deadbeef")
    monkeypatch.setenv("VALE", "false")
    command = action_entry._command(["docs/release notes.md"])
    assert command[-5:] == [
        "--diff-base",
        "deadbeef",
        "--format",
        "json",
        "docs/release notes.md",
    ]


def test_changed_only_missing_merge_base_fails_closed(tmp_path, monkeypatch, capsys):
    monkeypatch.setenv("CHANGED_ONLY", "true")
    monkeypatch.setenv("INPUT_PATHS", ".")
    monkeypatch.setenv("BASE_SHA", "missing")
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))

    def missing_base(command, **kwargs):
        return CompletedProcess(command, 128, stdout=b"", stderr=b"missing object")

    with patch("scripts.action_entry.subprocess.run", side_effect=missing_base):
        assert action_entry.resolve_targets() == 1
    assert "merge base" in capsys.readouterr().out


HOSTILE_NAMES = [
    ("a.md\n::error::forged.md", "a.md%0A::error::forged.md"),
    ("a.md\r::add-mask::secret.md", "a.md%0D::add-mask::secret.md"),
    ("a.md\n::stop-commands::tok.md", "a.md%0A::stop-commands::tok.md"),
    ("::error::lead.md", "::error::lead.md"),
    ("100%0A.md", "100%250A.md"),
    ("bell\x07esc\x1b.md", "bell%07esc%1B.md"),
]


@pytest.mark.parametrize(("name", "logged"), HOSTILE_NAMES)
def test_changed_file_names_are_escaped_in_log_output(
    tmp_path, monkeypatch, capsys, name, logged
):
    monkeypatch.setenv("CHANGED_ONLY", "true")
    monkeypatch.setenv("BASE_SHA", "abc123")
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))

    def fake_run(command, **kwargs):
        if command[:3] == ["git", "cat-file", "-e"]:
            return CompletedProcess(command, 0, stdout="", stderr="")
        return CompletedProcess(command, 0, stdout=name.encode() + b"\0", stderr=b"")

    with patch("scripts.action_entry.subprocess.run", side_effect=fake_run):
        assert action_entry.resolve_targets() == 0

    lines = capsys.readouterr().out.splitlines()
    assert lines == [f"- {logged}"]
    assert not any(line.lstrip().startswith("::") for line in lines)


EMPTY_REPORT = {
    "summary": {
        "score": 100,
        "findings": 0,
        "errors": 0,
        "warnings": 0,
        "suggestions": 0,
        "documents": 1,
        "words": 10,
        "per_100_words": 0.0,
        "passed": True,
        "categories": [],
    },
    "documents": [],
}


@pytest.mark.parametrize(("name", "logged"), HOSTILE_NAMES)
def test_resolved_target_names_reach_the_linter_unchanged(
    tmp_path, monkeypatch, name, logged
):
    for key, value in {
        "CHANGED_ONLY": "true",
        "BASE_SHA": "abc123",
        "RUNNER_TEMP": str(tmp_path),
        "GITHUB_OUTPUT": str(tmp_path / "output"),
        "GITHUB_STEP_SUMMARY": str(tmp_path / "summary"),
        "VALE": "false",
    }.items():
        monkeypatch.setenv(key, value)

    def fake_git(command, **kwargs):
        if command[:3] == ["git", "cat-file", "-e"]:
            return CompletedProcess(command, 0, stdout="", stderr="")
        return CompletedProcess(command, 0, stdout=name.encode() + b"\0", stderr=b"")

    with patch("scripts.action_entry.subprocess.run", side_effect=fake_git):
        assert action_entry.resolve_targets() == 0

    # Feed the path list resolve_targets() actually wrote into lint().
    monkeypatch.setenv("PATH_LIST", str(tmp_path / "slopvac-paths"))
    argvs: list[list[str]] = []

    def fake_linter(command, **kwargs):
        argvs.append(command)
        kwargs["stdout"].write(json.dumps(EMPTY_REPORT))
        return CompletedProcess(command, 0, stderr="")

    with patch("scripts.action_entry.subprocess.run", side_effect=fake_linter):
        assert action_entry.lint() == 0

    assert argvs
    assert all(argv[-1] == name for argv in argvs)


@pytest.mark.parametrize(
    ("name", "message", "expected"),
    [
        (
            "a.md\n::error::injected",
            "msg",
            (
                "::warning file=a.md%0A%3A%3Aerror%3A%3Ainjected,line=3,col=2,"
                "title=style.rule::msg"
            ),
        ),
        (
            "a.md\r::add-mask::x",
            "msg",
            (
                "::warning file=a.md%0D%3A%3Aadd-mask%3A%3Ax,line=3,col=2,"
                "title=style.rule::msg"
            ),
        ),
        (
            "a,b.md",
            "msg",
            "::warning file=a%2Cb.md,line=3,col=2,title=style.rule::msg",
        ),
        (
            "c:d.md",
            "msg",
            "::warning file=c%3Ad.md,line=3,col=2,title=style.rule::msg",
        ),
        (
            "plain.md",
            "100% sure\n::error::x",
            (
                "::warning file=plain.md,line=3,col=2,title=style.rule"
                "::100%25 sure%0A::error::x"
            ),
        ),
    ],
)
def test_annotations_are_built_from_json_with_escaped_values(
    tmp_path, monkeypatch, capsys, name, message, expected
):
    path_list = tmp_path / "targets"
    path_list.write_bytes(name.encode() + b"\0")
    report = {
        "summary": {
            "score": 90.0,
            "findings": 1,
            "errors": 0,
            "warnings": 1,
            "suggestions": 0,
            "documents": 1,
            "words": 100,
            "per_100_words": 1.0,
            "passed": True,
            "categories": [],
        },
        "documents": [
            {
                "path": name,
                "findings": [
                    {
                        "path": name,
                        "line": 3,
                        "column": 2,
                        "rule_id": "style.rule",
                        "severity": "warning",
                        "message": message,
                    }
                ],
            }
        ],
    }
    for key, value in {
        "PATH_LIST": str(path_list),
        "GITHUB_OUTPUT": str(tmp_path / "output"),
        "GITHUB_STEP_SUMMARY": str(tmp_path / "summary"),
        "RUNNER_TEMP": str(tmp_path),
        "VALE": "false",
        "ANNOTATE": "true",
        "SARIF": "false",
    }.items():
        monkeypatch.setenv(key, value)
    argvs: list[list[str]] = []

    def fake_run(command, **kwargs):
        argvs.append(command)
        kwargs["stdout"].write(json.dumps(report))
        return CompletedProcess(command, 0, stderr="")

    with patch("scripts.action_entry.subprocess.run", side_effect=fake_run):
        assert action_entry.lint() == 0

    # The linter receives the exact, unescaped name.
    assert all(name in argv for argv in argvs)
    lines = capsys.readouterr().out.splitlines()
    assert lines == [
        expected,
        "::notice title=slopvac::score 90.0/100, 1 finding(s), 1.00 per 100 words",
    ]


def test_hostile_severity_and_rule_id_cannot_forge_commands(capsys):
    report = {
        **EMPTY_REPORT,
        "documents": [
            {
                "path": "a.md",
                "findings": [
                    {
                        "path": "a.md",
                        "line": 1,
                        "rule_id": "r\n::error::x",
                        "severity": "error\n::stop-commands::",
                        "message": "m",
                    }
                ],
            }
        ],
    }

    lines = action_entry._annotations(report)

    assert lines[0] == "::notice file=a.md,line=1,col=1,title=r%0A%3A%3Aerror%3A%3Ax::m"
    assert len(lines) == 2
