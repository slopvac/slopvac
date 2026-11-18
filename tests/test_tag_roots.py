"""check_tag_roots.py: tags must share slopvac's single root commit.

Each test builds throwaway git repos, so the real root commit is not needed: the
script takes the expected root from --root.
"""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "check_tag_roots.py"

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


@pytest.mark.parametrize("mode", [[], ["--pre-push"]], ids=["ci", "hook"])
def test_commit_with_two_roots_fails(repo, mode):
    git(
        repo["path"], "merge", "-q", "--allow-unrelated-histories", "-m", "m", "foreign"
    )
    git(repo["path"], "tag", "v1")
    result = run(repo, *mode)
    assert result.returncode == 1
    assert "  v1" in result.stderr


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


def test_hook_refuses_when_the_second_of_two_tags_is_foreign(repo):
    git(repo["path"], "tag", "v1", repo["tip"])
    git(repo["path"], "tag", "v2", repo["foreign"])
    result = run(repo, "--pre-push")
    assert result.returncode == 1
    assert "  v2" in result.stderr
    assert "  v1" not in result.stderr


def test_hook_checks_local_tags_whatever_the_push_carries(repo):
    git(repo["path"], "tag", "old-foreign", repo["foreign"])
    result = run(repo, "--pre-push", stdin="refs/heads/main abc refs/heads/main def\n")
    assert result.returncode == 1
    assert "  old-foreign" in result.stderr


def test_hook_passes_with_only_good_tags_and_no_new_commits(repo):
    git(repo["path"], "tag", "v1", repo["tip"])
    git(repo["path"], "tag", "-a", "-m", "x", "v2", repo["root"])
    result = run(repo, "--pre-push")
    assert result.returncode == 0
    assert "checked 2 tags: 0 offending" in result.stdout


def test_hook_passes_without_tags(repo):
    assert run(repo, "--pre-push").returncode == 0


def test_hook_names_the_command_that_deletes_a_few_tags(repo):
    git(repo["path"], "tag", "pi-v1", repo["foreign"])
    git(repo["path"], "tag", "pi-v2", repo["foreign"])
    result = run(repo, "--pre-push")
    assert "git tag -d pi-v1 pi-v2" in result.stderr


def test_hook_names_a_filtering_command_for_many_tags(repo):
    for i in range(11):
        git(repo["path"], "tag", f"t{i:02}", repo["foreign"])
    result = run(repo, "--pre-push")
    assert "xargs git tag -d" in result.stderr
    assert repo["root"] in result.stderr
    assert "git tag -d t00" not in result.stderr


def test_ci_mode_does_not_print_a_delete_command(repo):
    git(repo["path"], "tag", "pi-v1", repo["foreign"])
    assert "git tag -d" not in run(repo).stderr


def test_default_root_is_slopvacs_real_root(repo):
    """Without --root a fixture repo fails, and the message names slopvac's root."""
    git(repo["path"], "tag", "v1", repo["tip"])
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        cwd=repo["path"],
        capture_output=True,
        text=True,
        check=False,
        env={k: v for k, v in os.environ.items() if k != "SLOPVAC_ROOT_COMMIT"},
    )
    assert result.returncode == 1
    assert "b516adb5ed8789b2b737c5876f75f2fbc809e755" in result.stderr


# --- shallow clones --------------------------------------------------------


def workflow_fetch_command() -> list[str]:
    """The workflow's tag fetch step, split into arguments."""
    workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "tag-roots.yml").read_text()
    )
    steps = workflow["jobs"]["tag-roots"]["steps"]
    command = next(s["run"] for s in steps if s.get("run", "").startswith("git fetch"))
    return shlex.split(command)[1:]


@pytest.fixture
def shallow_clone(repo, tmp_path) -> Path:
    """A depth-1 clone, like actions/checkout's default, with a tag at HEAD."""
    git(repo["path"], "tag", "v1", repo["tip"])
    git(repo["path"], "config", "uploadpack.allowFilter", "true")
    clone = tmp_path / "clone"
    git(tmp_path, "clone", "-q", "--depth", "1", repo["path"].as_uri(), str(clone))
    return clone


def test_shallow_clone_is_reported_as_incomplete_not_foreign(repo, shallow_clone):
    result = run({**repo, "path": shallow_clone})
    assert result.returncode == 2
    assert "shallow" in result.stderr
    assert "v1" not in result.stderr


def test_workflow_fetch_makes_a_shallow_checkout_checkable(repo, shallow_clone):
    git(shallow_clone, *workflow_fetch_command())
    result = run({**repo, "path": shallow_clone})
    assert result.returncode == 0, result.stderr
    assert "checked 1 tags: 0 offending" in result.stdout
