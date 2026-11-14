"""Behaviour of the Vale pin guard, run in isolated pytest subprocesses."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

pytest_plugins = ["pytester"]

GUARD_DIR = Path(__file__).resolve().parent

_TESTS = """
import pytest

@pytest.mark.vale
def test_needs_vale():
    pass

def test_plain():
    pass
"""

_CONFTEST = """
from vale_pin_guard import pytest_collection_modifyitems, pytest_configure
"""


def _fake_vale(bindir: Path, version: str) -> None:
    bindir.mkdir(parents=True, exist_ok=True)
    binary = bindir / "vale"
    binary.write_text(f'#!/bin/sh\necho "vale version {version}"\n', encoding="utf-8")
    binary.chmod(0o755)


@pytest.fixture
def guarded(pytester: pytest.Pytester, monkeypatch: pytest.MonkeyPatch):
    """A project with the guard hooks, a pinned mise.toml, and two tests."""
    pytester.makeconftest(_CONFTEST)
    pytester.makepyfile(test_suite=_TESTS)
    pytester.makefile(".toml", mise='[tools]\nvale = "3.24.0"\n')
    monkeypatch.setenv("PYTHONPATH", str(GUARD_DIR))
    return pytester


def _use_vale(pytester: pytest.Pytester, monkeypatch, version: str) -> None:
    bindir = pytester.path / "bin"
    _fake_vale(bindir, version)
    monkeypatch.setenv("PATH", f"{bindir}{os.pathsep}{os.environ['PATH']}")


def _use_no_vale(pytester: pytest.Pytester, monkeypatch) -> None:
    empty = pytester.path / "empty-bin"
    empty.mkdir()
    monkeypatch.setenv("PATH", str(empty))


def test_matching_version_runs_vale_tests(guarded, monkeypatch):
    _use_vale(guarded, monkeypatch, "3.24.0")

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider")

    result.assert_outcomes(passed=2)


def test_mismatch_rejects_run_selecting_vale_tests(guarded, monkeypatch):
    _use_vale(guarded, monkeypatch, "3.27.0")

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider")

    assert result.ret == pytest.ExitCode.USAGE_ERROR
    result.stderr.fnmatch_lines(
        ["*vale 3.27.0 is on PATH but mise.toml pins 3.24.0*mise exec -- uv run pytest*"]
    )


def test_mismatch_still_runs_unrelated_selection(guarded, monkeypatch):
    _use_vale(guarded, monkeypatch, "3.27.0")

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider", "-k", "plain")

    result.assert_outcomes(passed=1, deselected=1)


def test_absent_vale_skips_vale_tests_without_reading_pin(guarded, monkeypatch):
    _use_no_vale(guarded, monkeypatch)
    (guarded.path / "mise.toml").unlink()

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider", "-rs")

    result.assert_outcomes(passed=1, skipped=1)
    result.stdout.fnmatch_lines(["*SKIPPED*vale is not on PATH*"])


def test_missing_pin_is_a_usage_error_when_enforcement_is_needed(guarded, monkeypatch):
    _use_vale(guarded, monkeypatch, "3.24.0")
    (guarded.path / "mise.toml").unlink()

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider")

    assert result.ret == pytest.ExitCode.USAGE_ERROR
    result.stderr.fnmatch_lines(["*cannot find mise.toml*"])


def test_unparseable_pin_is_a_usage_error(guarded, monkeypatch):
    _use_vale(guarded, monkeypatch, "3.24.0")
    (guarded.path / "mise.toml").write_text("[tools\nvale = \n", encoding="utf-8")

    result = guarded.runpytest_subprocess("-p", "no:cacheprovider")

    assert result.ret == pytest.ExitCode.USAGE_ERROR
    result.stderr.fnmatch_lines(["*mise.toml is not valid TOML*"])
