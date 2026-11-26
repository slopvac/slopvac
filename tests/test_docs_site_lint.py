"""The docs-site workflow's lint step runs the 3.x binary with a 3.x config.

`docs-site.yml` lints the generated reference pages. The step runs here through its real
`run:` text against a stub binary, so no build is needed. Expected values come from the
exit-code contract (0 pass, 1 gate failed, 2 incomplete) and from the 3.x config schema.
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
NO_PAGES = "Docs site lint: no generated pages found"


def _steps() -> list[dict]:
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["build"]["steps"]


def _step(name: str) -> dict:
    (step,) = [s for s in _steps() if s.get("name") == name]
    return step


def _run_lint_step(tmp_path: Path, code: int, pages: str) -> subprocess.CompletedProcess:
    """Run the lint step's script with a stub binary that records its arguments.

    `pages` is "page" (one generated page), "empty" (a directory without pages) or
    "missing" (no directory at all).
    """
    stub = tmp_path / "target" / "debug" / "slopvac"
    stub.parent.mkdir(parents=True)
    stub.write_text(
        f'#!/bin/sh\nprintf "%s\\n" "$@" > "{tmp_path}/args"\nexit {code}\n',
        encoding="utf-8",
    )
    stub.chmod(0o755)
    site = tmp_path / "site-src"
    if pages != "missing":
        site.mkdir()
    if pages == "page":
        (site / "index.md").write_text("# Page\n", encoding="utf-8")
    env = {"PATH": "/usr/bin:/bin", "RUNNER_TEMP": str(tmp_path)}
    return subprocess.run(
        ["bash", "-c", _step(LINT)["run"]],
        cwd=tmp_path,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )


@pytest.mark.parametrize("pages", ["empty", "missing"])
def test_a_run_without_generated_pages_fails_with_a_fixed_line(tmp_path, pages) -> None:
    result = _run_lint_step(tmp_path, 0, pages)
    assert result.returncode == 1
    assert NO_PAGES in result.stdout
    assert str(tmp_path) not in result.stdout + result.stderr
    assert not (tmp_path / "args").exists()


def test_a_run_with_a_page_runs_the_linter(tmp_path) -> None:
    result = _run_lint_step(tmp_path, 0, "page")
    assert result.returncode == 0
    assert NO_PAGES not in result.stdout
    assert (tmp_path / "args").exists()


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
    code = [ln for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")]
    assert code[0] == "schema_version = 3"


def test_no_step_calls_the_2x_linter() -> None:
    for step in _steps():
        run = step.get("run", "")
        assert "packages/slopvac-lint" not in run, step.get("name")
        assert "install_vale" not in run, step.get("name")


@pytest.mark.parametrize("code", [0, 1, 2], ids=["pass", "gate-failed", "incomplete"])
def test_the_lint_step_exits_with_the_binary_exit_code(tmp_path, code) -> None:
    assert _run_lint_step(tmp_path, code, "page").returncode == code
