"""The real-binary fixtures fail in CI instead of skipping."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "slopvac_conftest", Path(__file__).with_name("conftest.py")
)
assert SPEC is not None and SPEC.loader is not None
conftest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(conftest)


def make_binary(root: Path, profile: str) -> Path:
    binary = root / "target" / profile / "slopvac"
    binary.parent.mkdir(parents=True)
    binary.write_text("", encoding="utf-8")
    return binary


@pytest.mark.parametrize("env", [{"CI": "true"}, {"GITHUB_ACTIONS": "true"}])
def test_a_missing_binary_fails_in_ci(tmp_path, env):
    with pytest.raises(pytest.fail.Exception, match="no built slopvac binary in CI"):
        conftest.locate_binary(env, tmp_path)


def test_a_missing_binary_is_left_to_the_caller_to_skip_outside_ci(tmp_path):
    assert conftest.locate_binary({}, tmp_path) is None


@pytest.mark.parametrize("env", [{}, {"CI": "true"}])
def test_a_declared_path_that_is_not_a_file_always_fails(tmp_path, env):
    env = {**env, "SLOPVAC_BIN": str(tmp_path / "nowhere")}
    with pytest.raises(pytest.fail.Exception, match="is not a file"):
        conftest.locate_binary(env, tmp_path)


def test_the_declared_binary_wins_over_the_target_directory(tmp_path):
    make_binary(tmp_path, "debug")
    declared = tmp_path / "mine"
    declared.write_text("", encoding="utf-8")
    assert conftest.locate_binary({"SLOPVAC_BIN": str(declared)}, tmp_path) == declared


def test_the_release_build_is_preferred_to_the_debug_build(tmp_path):
    make_binary(tmp_path, "debug")
    release = make_binary(tmp_path, "release")
    assert conftest.locate_binary({"CI": "true"}, tmp_path) == release
