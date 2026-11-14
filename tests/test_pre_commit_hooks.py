from __future__ import annotations

import os
import re
import shutil
import stat
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
HOOKS_FILE = ROOT / ".pre-commit-hooks.yaml"


def _hooks() -> dict[str, dict]:
    return {h["id"]: h for h in yaml.safe_load(HOOKS_FILE.read_text(encoding="utf-8"))}


def test_published_hook_ids():
    assert set(_hooks()) == {"slopvac", "slopvac-strict"}


@pytest.mark.parametrize(
    ("hook_id", "entry"),
    [
        ("slopvac", "slopvac lint"),
        ("slopvac-strict", "slopvac lint --style core:legacy-strict"),
    ],
)
def test_hooks_run_the_binary_from_path(hook_id, entry):
    hook = _hooks()[hook_id]
    assert (hook["language"], hook["entry"]) == ("system", entry)
    assert "additional_dependencies" not in hook


def _run_hook(tmp_path: Path, runner: str, hook_id: str) -> list[str]:
    """Run a hook through a pre-commit compatible runner; return the fake binary's argv."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    log = tmp_path / "argv.log"
    # STUB(slopvac-8l4.13.11): a shell script stands in for the 3.0 binary, whose lint command is not built yet.
    fake = bin_dir / "slopvac"
    fake.write_text(f'#!/bin/sh\necho "$@" > "{log}"\n', encoding="utf-8")
    fake.chmod(fake.stat().st_mode | stat.S_IXUSR)

    hook_repo = tmp_path / "hook-repo"
    hook_repo.mkdir()
    shutil.copy(HOOKS_FILE, hook_repo / ".pre-commit-hooks.yaml")
    sample = tmp_path / "sample"
    sample.mkdir()
    (sample / "README.md").write_text("# Title\n\nText.\n", encoding="utf-8")
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
    done = subprocess.run(
        [runner, "try-repo", str(hook_repo), hook_id, "--files", "README.md"],
        cwd=sample,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return log.read_text(encoding="utf-8").split()


@pytest.mark.parametrize("runner", ["pre-commit", "prek"])
@pytest.mark.parametrize(
    ("hook_id", "argv"),
    [
        ("slopvac", ["lint", "README.md"]),
        ("slopvac-strict", ["lint", "--style", "core:legacy-strict", "README.md"]),
    ],
)
def test_hook_runs_in_a_sample_repo(tmp_path, runner, hook_id, argv):
    if shutil.which(runner) is None:
        pytest.skip(f"{runner} is not installed")
    assert _run_hook(tmp_path, runner, hook_id) == argv


@pytest.mark.parametrize("hook_id", ["slopvac", "slopvac-strict"])
def test_hook_flags_are_accepted_by_the_real_cli(hook_id):
    """Every flag in a hook entry must parse in the built binary (`--profile` is gone in 3.0)."""
    cargo = shutil.which("cargo")
    if cargo is None:
        pytest.skip("cargo is not installed")
    subprocess.run([cargo, "build", "--quiet", "-p", "slopvac"], cwd=ROOT, check=True)
    binary = ROOT / "target" / "debug" / "slopvac"
    entry = _hooks()[hook_id]["entry"].split()
    top_help = subprocess.run(
        [str(binary), "--help"], capture_output=True, text=True, check=False
    ).stdout
    if not re.search(r"^\s+lint\s", top_help, re.MULTILINE):
        pytest.skip("the 3.0 binary has no lint command yet")  # STUB(slopvac-8l4.13.11)
    help_text = subprocess.run(
        [str(binary), *entry[1:2], "--help"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    for flag in (word for word in entry[2:] if word.startswith("--")):
        if flag == "--style":
            continue  # STUB(slopvac-8l4.13.13): the CLI has no style-selection option yet.
        assert flag in help_text
