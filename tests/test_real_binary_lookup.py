"""The real-binary fixtures fail instead of skipping when the binary is required."""

from __future__ import annotations

from pathlib import Path

import pytest
import real_binary


def make_binary(root: Path, profile: str) -> Path:
    binary = root / "target" / profile / "slopvac"
    binary.parent.mkdir(parents=True)
    binary.write_text("", encoding="utf-8")
    return binary


def test_a_missing_binary_fails_when_required(tmp_path):
    with pytest.raises(pytest.fail.Exception, match="SLOPVAC_REQUIRE_BIN=1"):
        real_binary.locate_binary({"SLOPVAC_REQUIRE_BIN": "1"}, tmp_path)


def test_a_missing_binary_is_left_to_the_caller_to_skip_when_not_required(tmp_path):
    assert real_binary.locate_binary({}, tmp_path) is None


@pytest.mark.parametrize("env", [{"CI": "true"}, {"GITHUB_ACTIONS": "true"}])
def test_a_generic_ci_marker_does_not_make_the_binary_required(tmp_path, env):
    # Only SLOPVAC_REQUIRE_BIN opts in; other CI jobs that run pytest must still skip.
    assert real_binary.locate_binary(env, tmp_path) is None


@pytest.mark.parametrize("env", [{}, {"SLOPVAC_REQUIRE_BIN": "1"}])
def test_a_declared_path_that_is_not_a_file_always_fails(tmp_path, env):
    env = {**env, "SLOPVAC_BIN": str(tmp_path / "nowhere")}
    with pytest.raises(pytest.fail.Exception, match="is not a file"):
        real_binary.locate_binary(env, tmp_path)


def test_the_declared_binary_wins_over_the_target_directory(tmp_path):
    make_binary(tmp_path, "debug")
    declared = tmp_path / "mine"
    declared.write_text("", encoding="utf-8")
    assert (
        real_binary.locate_binary({"SLOPVAC_BIN": str(declared)}, tmp_path) == declared
    )


def test_the_release_build_is_preferred_to_the_debug_build(tmp_path):
    make_binary(tmp_path, "debug")
    release = make_binary(tmp_path, "release")
    assert real_binary.locate_binary({"SLOPVAC_REQUIRE_BIN": "1"}, tmp_path) == release
