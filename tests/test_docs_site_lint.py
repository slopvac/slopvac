"""The docs-site workflow's lint step runs the 3.x binary with a 3.x config.

`docs-site.yml` lints the generated reference pages. The step runs here through its real
`run:` text against controlled doubles, so no build is needed and the outcome never
depends on what is installed:

* a stub `slopvac` at `target/debug/slopvac`, the path the build step produces, which
  records each invocation and exits with a chosen code;
* a stub `uvx` on `PATH` that exits 99, so a step that still launches the 2.x linter
  through it fails an assertion instead of a missing tool.

Expected values come from the exit-code contract (0 pass, 1 gate failed, 2 incomplete)
and from the 3.x config schema.
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
BUILT_BINARY = "target/debug/slopvac"
SEP = "\x1f"
UVX_EXIT = 99


def _steps() -> list[dict]:
    jobs = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]
    return jobs["build"]["steps"]


def _step(name: str) -> dict:
    found = [s for s in _steps() if s.get("name") == name]
    assert len(found) == 1, f"expected one step named {name!r}, found {len(found)}"
    return found[0]


def _stub(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n" + body, encoding="utf-8")
    path.chmod(0o755)


def _run_lint_step(
    tmp_path: Path, code: int, pages: str
) -> subprocess.CompletedProcess:
    """Run the lint step's script with doubles for both launchers.

    `pages` is "page" (one generated page), "empty" (a directory without pages),
    "other" (a directory with only a non-Markdown file) or "missing" (no directory).
    """
    calls = tmp_path / "calls"
    _stub(
        tmp_path / BUILT_BINARY,
        f'printf "%s" "$0" >> "{calls}"\n'
        f'for a in "$@"; do printf "\\037%s" "$a" >> "{calls}"; done\n'
        f'echo >> "{calls}"\nexit {code}\n',
    )
    _stub(tmp_path / "bin" / "uvx", f"exit {UVX_EXIT}\n")
    site = tmp_path / "site-src"
    if pages != "missing":
        site.mkdir()
    if pages == "page":
        (site / "index.md").write_text("# Page\n", encoding="utf-8")
    if pages == "other":
        (site / "notes.txt").write_text("not a page\n", encoding="utf-8")
    env = {"PATH": f"{tmp_path / 'bin'}:/usr/bin:/bin", "RUNNER_TEMP": str(tmp_path)}
    return subprocess.run(
        ["bash", "-c", _step(LINT)["run"]],
        cwd=tmp_path,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )


def _invocation(tmp_path: Path) -> list[str]:
    """Return argv (with argv[0]) of the one recorded call of the stub binary."""
    calls = tmp_path / "calls"
    assert calls.exists(), "the step never ran the stub binary"
    lines = calls.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 1, f"expected one invocation, found {len(lines)}"
    return lines[0].split(SEP)


@pytest.mark.parametrize("pages", ["empty", "other", "missing"])
def test_a_run_without_generated_pages_fails_with_a_fixed_line(tmp_path, pages) -> None:
    result = _run_lint_step(tmp_path, 0, pages)
    assert result.returncode == 1
    assert NO_PAGES in result.stdout
    assert str(tmp_path) not in result.stdout + result.stderr
    assert not (tmp_path / "calls").exists()


def test_a_run_with_a_page_runs_the_linter(tmp_path) -> None:
    result = _run_lint_step(tmp_path, 0, "page")
    assert result.returncode == 0
    assert NO_PAGES not in result.stdout
    _invocation(tmp_path)


def test_the_build_step_comes_before_the_lint_step() -> None:
    names = [s.get("name") for s in _steps()]
    assert BUILD in names, f"no step named {BUILD!r}"
    assert LINT in names, f"no step named {LINT!r}"
    assert names.index(BUILD) < names.index(LINT)


def test_the_step_runs_the_binary_that_the_build_produces(tmp_path) -> None:
    _run_lint_step(tmp_path, 0, "page")
    assert _invocation(tmp_path)[0] == BUILT_BINARY


def test_the_site_config_is_a_3x_config() -> None:
    text = CONFIG.read_text(encoding="utf-8")
    config = tomllib.loads(text)
    assert config.get("schema_version") == 3, "site-lint.toml is not on schema 3"
    assert config.get("profile") == "relaxed", "site-lint.toml sets no relaxed profile"
    assert config.get("styles") == ["core:legacy-relaxed"], (
        "site-lint.toml lists no styles"
    )
    assert config.get("thresholds") == {"max_unicode_dashes": 1}
    code = [
        ln for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")
    ]
    assert code[0] == "schema_version = 3"


@pytest.mark.parametrize("code", [0, 1, 2], ids=["pass", "gate-failed", "incomplete"])
def test_the_lint_step_exits_with_the_binary_exit_code(tmp_path, code) -> None:
    assert _run_lint_step(tmp_path, code, "page").returncode == code


@pytest.mark.parametrize("code", [0, 1, 2], ids=["pass", "gate-failed", "incomplete"])
def test_the_lint_step_prints_the_real_exit_code(tmp_path, code) -> None:
    assert f"slopvac exited {code}" in _run_lint_step(tmp_path, code, "page").stdout


def test_the_binary_gets_the_page_directory_and_the_site_config(tmp_path) -> None:
    _run_lint_step(tmp_path, 0, "page")
    assert _invocation(tmp_path)[1:] == [
        "lint",
        str(tmp_path / "site-src"),
        "--config",
        "docs/site-lint.toml",
    ]
