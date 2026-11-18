"""check_tag_roots.py: tags must share slopvac's single root commit.

Each test builds throwaway git repos, so the real root commit is not needed: the
script takes the expected root from --root.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "check_tag_roots.py"
ZERO = "0" * 40

GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.com",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.com",
    "GIT_CONFIG_GLOBAL": "/dev/null",
    "GIT_CONFIG_SYSTEM": "/dev/null",
}


def git(repo: Path, *args: str, stdin: str | None = None) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        input=stdin,
        capture_output=True,
        text=True,
        check=True,
        env={**os.environ, **GIT_ENV},
    )
    return result.stdout.strip()


def commit(repo: Path, message: str) -> str:
    git(repo, "commit", "-q", "--allow-empty", "-m", message)
    return git(repo, "rev-parse", "HEAD")


@pytest.fixture
def repo(tmp_path: Path) -> dict:
    """A repo with a main history and a second, unrelated history."""
    path = tmp_path / "repo"
    path.mkdir()
    git(path, "init", "-q", "-b", "main")
    root = commit(path, "root")
    tip = commit(path, "tip")
    git(path, "checkout", "-q", "--orphan", "foreign")
    foreign = commit(path, "foreign root")
    git(path, "checkout", "-q", "main")
    return {"path": path, "root": root, "tip": tip, "foreign": foreign}


def run(repo: dict, *args: str, stdin: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", repo["root"], *args],
        cwd=repo["path"],
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    )


def push_line(ref: str, sha: str, remote_sha: str = ZERO) -> str:
    return f"{ref} {sha} {ref} {remote_sha}\n"


def test_tag_on_the_expected_history_passes(repo):
    git(repo["path"], "tag", "v1", repo["tip"])
    result = run(repo)
    assert result.returncode == 0
    assert "checked 1 tags: 0 offending" in result.stdout


def test_tag_on_an_unrelated_history_fails_and_is_named(repo):
    git(repo["path"], "tag", "v1", repo["tip"])
    git(repo["path"], "tag", "pi-v9", repo["foreign"])
    result = run(repo)
    assert result.returncode == 1
    assert "1 of 2 tags" in result.stderr
    assert "  pi-v9" in result.stderr
    assert "  v1" not in result.stderr


def test_annotated_tag_is_peeled_to_its_commit(repo):
    git(repo["path"], "tag", "-a", "-m", "ok", "v1", repo["tip"])
    git(repo["path"], "tag", "-a", "-m", "bad", "v2", repo["foreign"])
    result = run(repo)
    assert result.returncode == 1
    assert "1 of 2 tags" in result.stderr
    assert "  v2" in result.stderr


def test_commit_with_two_roots_fails(repo):
    git(
        repo["path"], "merge", "-q", "--allow-unrelated-histories", "-m", "m", "foreign"
    )
    git(repo["path"], "tag", "v1")
    assert run(repo).returncode == 1


def test_failure_list_is_capped_at_fifty_names(repo):
    for i in range(52):
        git(repo["path"], "tag", f"t{i:02}", repo["foreign"])
    result = run(repo)
    assert "52 of 52 tags" in result.stderr
    assert result.stderr.count("\n  t") == 50
    assert "... and 2 more" in result.stderr


def test_not_a_repository_is_incomplete_not_failed(tmp_path):
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--root", "a" * 40],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
        env={"GIT_CEILING_DIRECTORIES": str(tmp_path.parent), "PATH": "/usr/bin:/bin"},
    )
    assert result.returncode == 2


# --- pre-push mode ---------------------------------------------------------


@pytest.mark.parametrize(
    ("line", "expected"),
    [
        pytest.param("tag_ok", 0, id="good-tag"),
        pytest.param("tag_bad", 1, id="foreign-tag"),
        pytest.param("annotated_bad", 1, id="annotated-foreign-tag"),
        pytest.param("annotated_ok", 0, id="annotated-good-tag"),
        pytest.param("delete", 0, id="deletion-ignored"),
        pytest.param("branch_bad", 0, id="branch-ignored"),
    ],
)
def test_pre_push_reads_stdin_lines(repo, line, expected):
    path = repo["path"]
    git(path, "tag", "-a", "-m", "x", "annotated_ok", repo["tip"])
    git(path, "tag", "-a", "-m", "x", "annotated_bad", repo["foreign"])
    lines = {
        "tag_ok": push_line("refs/tags/v1", repo["tip"]),
        "tag_bad": push_line("refs/tags/pi-v9", repo["foreign"]),
        "annotated_bad": push_line(
            "refs/tags/v2", git(path, "rev-parse", "annotated_bad")
        ),
        "annotated_ok": push_line(
            "refs/tags/v3", git(path, "rev-parse", "annotated_ok")
        ),
        "delete": push_line("refs/tags/pi-v9", ZERO, repo["foreign"]),
        "branch_bad": push_line("refs/heads/foreign", repo["foreign"]),
    }
    result = run(repo, "--pre-push", stdin=lines[line])
    assert result.returncode == expected, result.stderr
    if expected:
        assert "pi-v9" in result.stderr or "v2" in result.stderr


def test_pre_push_ignores_tags_that_are_not_being_pushed(repo):
    git(repo["path"], "tag", "old-foreign", repo["foreign"])
    line = push_line("refs/tags/v1", repo["tip"])
    assert run(repo, "--pre-push", stdin=line).returncode == 0


def test_pre_push_falls_back_to_the_ref_prek_exposes(repo):
    env = {
        **os.environ,
        "PRE_COMMIT_REMOTE_BRANCH": "refs/tags/pi-v9",
        "PRE_COMMIT_TO_REF": repo["foreign"],
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--root", repo["root"], "--pre-push"],
        cwd=repo["path"],
        input="",
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )
    assert result.returncode == 1
    assert "  pi-v9" in result.stderr


def test_default_root_is_slopvacs_real_root():
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            f"import runpy; print(runpy.run_path({str(SCRIPT)!r})['ROOT_COMMIT'])",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    assert result.stdout.strip() == "b516adb5ed8789b2b737c5876f75f2fbc809e755"
