"""The docs-site workflow's lint step runs the 3.x binary with a 3.x config.

`docs-site.yml` lints the generated reference pages. The step builds the binary from the
checkout and ends with its exit code, so exit 1 (a gate failed) and exit 2 (the run was
incomplete) both fail the step. The step script runs here against a stub binary, so no
build is needed. Expected values come from the exit-code contract (0 pass, 1 gate failed,
2 incomplete) and from the 3.x config schema.
"""

from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "docs-site.yml"
CONFIG = ROOT / "docs" / "site-lint.toml"
LINT = "Lint the generated Markdown"
BUILD = "Build the slopvac binary"


def _steps() -> list[dict]:
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["build"][
        "steps"
    ]


def _step(name: str) -> dict:
    (step,) = [s for s in _steps() if s.get("name") == name]
    return step


def test_no_step_calls_the_2x_linter() -> None:
    for step in _steps():
        run = step.get("run", "")
        assert "packages/slopvac-lint" not in run, step.get("name")
        assert "install_vale" not in run, step.get("name")


def test_the_lint_step_runs_the_built_binary_on_the_site_config() -> None:
    run = _step(LINT)["run"]
    assert "target/debug/slopvac lint" in run
    assert '"$RUNNER_TEMP/site-src"' in run
    assert "--config docs/site-lint.toml" in run
    assert "--profile" not in run


def test_the_binary_is_built_before_the_lint_step() -> None:
    names = [s.get("name") for s in _steps()]
    assert names.index(BUILD) < names.index(LINT)
    assert _step(BUILD)["run"].strip() == "cargo build --locked -p slopvac"


def test_the_site_config_is_a_3x_config() -> None:
    text = CONFIG.read_text(encoding="utf-8")
    config = tomllib.loads(text)
    assert config["schema_version"] == 3
    assert config["profile"] == "relaxed"
    assert config["thresholds"] == {"max_unicode_dashes": 1}
    code = [
        ln for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")
    ]
    assert code[0] == "schema_version = 3"


@pytest.mark.parametrize("code", [0, 1, 2], ids=["pass", "gate-failed", "incomplete"])
def test_the_lint_step_exits_with_the_binary_exit_code(tmp_path, code) -> None:
    stub = tmp_path / "target" / "debug" / "slopvac"
    stub.parent.mkdir(parents=True)
    stub.write_text(f"#!/bin/sh\nexit {code}\n", encoding="utf-8")
    stub.chmod(0o755)
    env = {"PATH": "/usr/bin:/bin", "RUNNER_TEMP": str(tmp_path)}
    result = subprocess.run(
        ["bash", "-c", _step(LINT)["run"]],
        cwd=tmp_path,
        env=env,
        check=False,
        capture_output=True,
    )
    assert result.returncode == code
