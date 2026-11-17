from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
HOOKS_FILE = ROOT / ".pre-commit-hooks.yaml"
RUNNERS = ["pre-commit", "prek"]

CLEAN = b"# Title\n\nText.\n"
BAD = (
    b"# Title\n\nIn today's fast-paced world, we delve into a rich tapestry of "
    b"seamless, game-changing synergy.\n"
)
NOT_UTF8 = b"\xff\xfebad\n"


def _hooks() -> dict[str, dict]:
    return {h["id"]: h for h in yaml.safe_load(HOOKS_FILE.read_text(encoding="utf-8"))}


def test_published_hook_ids():
    assert set(_hooks()) == {"slopvac"}


def test_hook_runs_the_binary_from_path():
    hook = _hooks()["slopvac"]
    assert (hook["language"], hook["entry"]) == ("system", "slopvac lint")
    assert "additional_dependencies" not in hook


@pytest.fixture(scope="session")
def slopvac_binary() -> Path:
    """Build the workspace CLI once; the hook tests run it through the runners."""
    cargo = shutil.which("cargo")
    assert cargo, "cargo is required to build the CLI under test"
    subprocess.run([cargo, "build", "--quiet", "-p", "slopvac"], cwd=ROOT, check=True)
    return ROOT / "target" / "debug" / "slopvac"


def _run_hook(
    tmp_path: Path, runner: str, hook_id: str, binary: Path, content: bytes
) -> subprocess.CompletedProcess[str]:
    """Run a hook through a pre-commit compatible runner with the real CLI on PATH."""
    assert shutil.which(runner), f"{runner} is required; install the dev extra"
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "slopvac").symlink_to(binary)

    hook_repo = tmp_path / "hook-repo"
    hook_repo.mkdir()
    shutil.copy(HOOKS_FILE, hook_repo / ".pre-commit-hooks.yaml")
    sample = tmp_path / "sample"
    sample.mkdir()
    (sample / "README.md").write_bytes(content)
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@example.com",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@example.com",
        "PRE_COMMIT_HOME": str(tmp_path / "cache"),
        "PREK_HOME": str(tmp_path / "cache"),
    }
    for repo in (hook_repo, sample):
        subprocess.run(["git", "init", "-q"], cwd=repo, check=True, env=env)
        subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=env)
        subprocess.run(["git", "commit", "-qm", "init"], cwd=repo, check=True, env=env)
    return subprocess.run(
        [runner, "try-repo", str(hook_repo), hook_id, "--files", "README.md"],
        cwd=sample,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize("runner", RUNNERS)
def test_hook_passes_on_clean_prose(tmp_path, runner, slopvac_binary):
    done = _run_hook(tmp_path, runner, "slopvac", slopvac_binary, CLEAN)
    assert done.returncode == 0, done.stdout + done.stderr
    assert "Passed" in done.stdout


@pytest.mark.parametrize("runner", RUNNERS)
def test_hook_fails_with_exit_2_when_the_check_is_incomplete(
    tmp_path, runner, slopvac_binary
):
    done = _run_hook(tmp_path, runner, "slopvac", slopvac_binary, NOT_UTF8)
    assert done.returncode != 0
    assert "exit code: 2" in done.stdout
    assert "not valid UTF-8" in done.stdout + done.stderr


@pytest.mark.parametrize("runner", RUNNERS)
def test_hook_fails_with_exit_1_on_findings(tmp_path, runner, slopvac_binary):
    done = _run_hook(tmp_path, runner, "slopvac", slopvac_binary, BAD)
    assert done.returncode != 0
    assert "exit code: 1" in done.stdout
    assert "README.md" in done.stdout
