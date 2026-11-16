"""The pinned Rust toolchain helper recovers from a partial install.

The script runs against a fake ``rustup`` on PATH that logs each call and fails
the first N ``toolchain install`` calls.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "install-rust-toolchain.sh"
WORKFLOWS = ROOT / ".github" / "workflows"
HELPER = "scripts/ci/install-rust-toolchain.sh"

FAKE_RUSTUP = """#!/usr/bin/env bash
echo "$*" >> "$RUSTUP_LOG"
if [[ "$1 $2" == "toolchain install" ]]; then
    n=$(( $(cat "$RUSTUP_COUNT" 2>/dev/null || echo 0) + 1 ))
    echo "$n" > "$RUSTUP_COUNT"
    if (( n <= RUSTUP_FAIL_FIRST )); then
        echo "detected conflict bin/cargo-clippy" >&2
        exit 1
    fi
fi
exit 0
"""


DEFAULT_TOML = '[toolchain]\nchannel = "9.8.7"\ncomponents = ["clippy"]\n'


def run_helper(
    tmp_path: Path, fail_first: int, toml: str = DEFAULT_TOML
) -> tuple[int, list[str]]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake = bin_dir / "rustup"
    fake.write_text(FAKE_RUSTUP)
    fake.chmod(0o755)
    repo = tmp_path / "repo"
    repo.mkdir()
    (repo / "rust-toolchain.toml").write_text(toml)
    log = tmp_path / "log"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "RUST_TOOLCHAIN_DIR": str(repo),
        "RUSTUP_LOG": str(log),
        "RUSTUP_COUNT": str(tmp_path / "count"),
        "RUSTUP_FAIL_FIRST": str(fail_first),
    }
    result = subprocess.run(
        ["bash", str(SCRIPT)], env=env, capture_output=True, text=True, check=False
    )
    calls = log.read_text().splitlines() if log.exists() else []
    return result.returncode, calls


def test_clean_install_does_not_uninstall(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=0)
    assert code == 0
    assert calls == ["toolchain install", "show active-toolchain"]


def test_partial_install_is_uninstalled_then_reinstalled(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=1)
    assert code == 0
    assert calls == [
        "toolchain install",
        "toolchain uninstall 9.8.7",
        "toolchain install",
        "show active-toolchain",
    ]


def test_failed_retry_exits_non_zero(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=2)
    assert code != 0
    assert "show active-toolchain" not in calls


@pytest.mark.parametrize(
    "toml",
    [
        '[toolchain]\nchannel = "9.8.7"\n',
        "[toolchain]\nchannel = '9.8.7'\n",
        '[toolchain]\nchannel = "9.8.7" # pinned\n',
        '[toolchain]\n  channel   =   "9.8.7"  \ncomponents = ["clippy"]\n',
    ],
    ids=["double", "single", "comment", "whitespace"],
)
def test_channel_is_parsed_as_toml(tmp_path: Path, toml: str) -> None:
    code, calls = run_helper(tmp_path, fail_first=1, toml=toml)
    assert code == 0
    assert "toolchain uninstall 9.8.7" in calls


def test_missing_channel_fails_before_rustup(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=0, toml="[toolchain]\n")
    assert code != 0
    assert calls == []


def test_malformed_toml_fails_before_rustup(tmp_path: Path) -> None:
    # A line-oriented regex would still find the channel and install it.
    toml = '[toolchain]\nchannel = "9.8.7"\n= broken\n'
    code, calls = run_helper(tmp_path, fail_first=0, toml=toml)
    assert code != 0
    assert calls == []


# Every job that checks out the repository installs the pinned toolchain first,
# because any cargo, rustc or rustfmt call under the checkout (from a script, a
# test or a hook) makes rustup auto-install it, and a partial preinstalled copy
# breaks that. A job without a repository checkout (a reusable-workflow call, an
# artifact-only or API-only job) has no pinned toolchain to read and is not
# checked.
#
# These jobs check out the repository but never run Rust. Add the helper to a
# new job, or list it here with a reason.
NO_RUST = {
    ("action-test.yml", "action"): "runs the Python action and pipx docutils",
    ("action-v3-test.yml", "action"): "runs the action against a fake binary",
    ("candidate.yml", "validate"): "tag parsing and key checks in Python",
    ("candidate.yml", "sign"): "minisign over downloaded artifacts",
    ("cli-e2e.yml", "e2e"): "installs the Python wheel; no cargo",
    ("cli-e2e.yml", "smoke"): "installs the Python package; no cargo",
    ("lint.yml", "markdownlint"): "npx markdownlint",
    ("lint.yml", "taplo"): "uvx taplo",
    ("lint.yml", "pinact"): "third-party pinact action",
    ("lint.yml", "complexity"): "uvx lizard over two Python files",
    ("lint.yml", "shell-lint"): "shellcheck",
    ("lint.yml", "python-lint"): "uvx ruff",
    ("lint.yml", "action"): "runs the Python action",
    ("lint.yml", "prose"): "slopvac lint (Python) over Markdown",
    ("markdown-audit.yml", "inventory-and-audit"): "Python Markdown audit",
    ("publish-lint.yml", "build"): "builds the Python sdist and wheel",
    ("release-provenance.yml", "provenance"): "tag and ruleset checks in Python",
    ("release-smoke.yml", "resolve"): "tag parsing in Python",
    ("release-smoke.yml", "archive"): "unpacks a released binary",
    ("release-smoke.yml", "shell-installer"): "runs the released shell installer",
    ("release-smoke.yml", "homebrew"): "brew install of the released formula",
    ("release.yml", "route"): "cargo-dist generated; tag routing in Python",
    ("release.yml", "plan"): "cargo-dist generated; dist plan only",
    ("release.yml", "build-local-artifacts"): "cargo-dist generated; installs rustup",
    ("release.yml", "build-global-artifacts"): "cargo-dist generated; own toolchain",
    ("release.yml", "host"): "cargo-dist generated; uploads artifacts",
    ("release.yml", "publish-homebrew-formula"): "cargo-dist generated; git only",
    ("release.yml", "announce"): "cargo-dist generated; no steps run",
    ("release.yml", "publish-legacy-release"): "release checks in Python",
    ("reviewdog.yml", "comment-v3"): "posts findings with reviewdog",
    ("security.yml", "dependency-review"): "third-party action",
    ("security.yml", "trufflehog"): "third-party action",
    ("security.yml", "codeql"): "CodeQL Python analysis",
    ("security.yml", "osv"): "third-party scanner",
    ("security.yml", "trivy"): "third-party scanner",
    ("test.yml", "changes"): "path classification",
    ("test.yml", "quick-lint"): "ruff, JSON checks and Python steering tests",
    ("trunk-dev-stale-branches.yml", "stale-bead-branches"): "bd and git only",
}
# Steps that may sit between checkout and the helper.
RUNTIME_SETUP = (
    "actions/setup-python@",
    "astral-sh/setup-uv@",
    "actions/setup-node@",
    "jdx/mise-action@",
)


def _all_jobs() -> list[tuple[str, str, dict]]:
    found = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        workflow = yaml.safe_load(path.read_text())
        for name, job in (workflow.get("jobs") or {}).items():
            found.append((path.name, name, job))
    return found


def _is_helper(step: dict) -> bool:
    return step.get("run", "").strip() == f"bash {HELPER}"


def _problem(file: str, name: str, job: dict) -> str | None:
    steps = job.get("steps") or []
    checkout = next(
        (
            i
            for i, s in enumerate(steps)
            if s.get("uses", "").startswith("actions/checkout@")
        ),
        None,
    )
    allowed = (file, name) in NO_RUST
    if checkout is None:
        return None
    has_helper = any(_is_helper(s) for s in steps)
    if allowed:
        return (
            f"{file}:{name} is in NO_RUST but runs the helper" if has_helper else None
        )
    rest = steps[checkout + 1 :]
    index = 0
    while index < len(rest) and rest[index].get("uses", "").startswith(RUNTIME_SETUP):
        index += 1
    if index < len(rest) and _is_helper(rest[index]):
        return None
    return (
        f"{file}:{name} must run `bash {HELPER}` right after checkout "
        "(runtime setup steps may come between), or be added to NO_RUST "
        "with a reason"
    )


@pytest.mark.parametrize(
    ("file", "name", "job"), _all_jobs(), ids=lambda v: v if isinstance(v, str) else ""
)
def test_job_installs_the_toolchain_or_is_classified(
    file: str, name: str, job: dict
) -> None:
    assert _problem(file, name, job) is None


def test_no_rust_entries_name_real_jobs() -> None:
    existing = {(f, n) for f, n, _ in _all_jobs()}
    assert set(NO_RUST) <= existing
