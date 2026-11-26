"""The repository's prose gate: the `prose` job's exit handling and the SARIF it exports.

`lint.yml` runs the 3.x binary on the repository's own docs. Two properties keep that
gate honest:

* The gate step ends with the binary's own exit code, so exit 1 (a gate failed) and exit 2
  (the run was incomplete or the invocation was invalid) both fail the step. Exit 2 is
  never a pass. The step scripts run here against a stub binary, so no build is needed.
* `reviewdog.yml` reads the exported SARIF with `-f=sarif`. The properties it relies on
  are checked against the real binary's output.

Expected values come from the exit-code contract (0 pass, 1 gate failed, 2 incomplete)
and from SARIF 2.1.0, not from the binary's current output.
"""

from __future__ import annotations

import copy
import json
import os
import re
import subprocess
from pathlib import Path

import pytest
import yaml
from real_binary import ROOT

pytest_plugins = ["real_binary"]  # the real-binary fixtures

LINT = ROOT / ".github" / "workflows" / "lint.yml"
GATE = "Gate this repo's own docs"
EXPORT = "Export prose findings"
FILE_SET = (
    "README.md",
    "packages/slopvac-lint/README.md",
    "packages/slopvac-lint/docs",
)


def _prose_job() -> dict:
    return yaml.safe_load(LINT.read_text(encoding="utf-8"))["jobs"]["prose"]


def _step(job: dict, name: str) -> dict:
    (step,) = [s for s in job["steps"] if s.get("name") == name]
    return step


def _truthy(value: object) -> bool:
    return value is True or str(value).lower() == "true"


# ---- the gate step -----------------------------------------------------------------


def _gate_problems(job: dict) -> list[str]:
    """Return every way the gate step can let a failing exit code through."""
    step = _step(job, GATE)
    script = step["run"]
    problems = []
    if _truthy(step.get("continue-on-error")) or _truthy(job.get("continue-on-error")):
        problems.append("continue-on-error is set")
    if re.search(r"\|\|\s*(true|:)(\s|$)", script) or re.search(r"\bset\s+\+e\b", script):
        problems.append("the script swallows the status")
    piped = re.search(r"(?<!\|)\|(?!\|)", script) is not None
    if piped and not re.search(r"set\s+-\w*o\s+pipefail|set\s+-\w*\s*-o\s+pipefail", script):
        problems.append("a pipe runs without pipefail")
    if not re.search(r'^\s*exit\s+"\$status"\s*$', script, re.MULTILINE):
        problems.append('the script does not end in exit "$status"')
    return problems


def _mutated(mutate) -> dict:
    job = copy.deepcopy(_prose_job())
    mutate(job, _step(job, GATE))
    return job


def _swallow(job, step):
    step["run"] = step["run"].replace("|| status=$?", "|| true")


def _continue(job, step):
    step["continue-on-error"] = True


def _continue_job(job, step):
    job["continue-on-error"] = True


def _no_exit(job, step):
    step["run"] = step["run"].replace('exit "$status"', "")


def _set_plus_e(job, step):
    step["run"] = "set +e\n" + step["run"]


def _pipe_without_pipefail(job, step):
    run = step["run"].replace("set -uo pipefail", "set -u")
    step["run"] = run.replace("\n", " | cat\n", 1)


@pytest.mark.parametrize(
    "mutate",
    [
        pytest.param(_swallow, id="or-true"),
        pytest.param(_continue, id="continue-on-error-step"),
        pytest.param(_continue_job, id="continue-on-error-job"),
        pytest.param(_no_exit, id="exit-removed"),
        pytest.param(_set_plus_e, id="set-plus-e"),
        pytest.param(_pipe_without_pipefail, id="pipe-without-pipefail"),
    ],
)
def test_a_gate_that_swallows_the_exit_code_is_reported(mutate) -> None:
    assert _gate_problems(_mutated(mutate)) != []


def test_the_gate_step_keeps_the_exit_code() -> None:
    assert _gate_problems(_prose_job()) == []


def _run_gate(script: str, tmp_path: Path, code: int) -> int:
    stub = tmp_path / "target" / "debug" / "slopvac"
    stub.parent.mkdir(parents=True)
    stub.write_text(f"#!/bin/sh\nexit {code}\n", encoding="utf-8")
    stub.chmod(0o755)
    return subprocess.run(
        ["bash", "-c", script], cwd=tmp_path, check=False, capture_output=True
    ).returncode


@pytest.mark.parametrize("code", [0, 1, 2], ids=["pass", "gate-failed", "incomplete"])
def test_the_gate_step_exits_with_the_binary_exit_code(tmp_path, code) -> None:
    script = _step(_prose_job(), GATE)["run"]
    assert _run_gate(script, tmp_path, code) == code


@pytest.mark.parametrize(
    "mutate",
    [
        pytest.param(_swallow, id="or-true"),
        pytest.param(_no_exit, id="exit-removed"),
    ],
)
def test_a_swallowing_script_lets_exit_2_pass(tmp_path, mutate) -> None:
    script = _step(_mutated(mutate), GATE)["run"]
    assert _run_gate(script, tmp_path, 2) != 2


def _paths_after(script: str) -> tuple[str, ...]:
    words = re.sub(r"\\\n", " ", script).split()
    start = words.index("lint") + 1
    paths = []
    for word in words[start:]:
        if word.startswith(("-", "|")):
            break
        paths.append(word)
    return tuple(paths)


def test_the_gate_and_the_export_lint_the_same_files() -> None:
    job = _prose_job()
    gate = _paths_after(_step(job, GATE)["run"])
    assert gate == FILE_SET
    assert _paths_after(_step(job, EXPORT)["run"]) == gate


# ---- the SARIF reviewdog reads ------------------------------------------------------

LEVELS = {"error", "warning", "note", "none"}


def _sarif_problems(sarif: dict) -> list[str]:
    problems = []
    if sarif.get("version") != "2.1.0":
        problems.append("version is not 2.1.0")
    for i, result in enumerate(sarif["runs"][0]["results"]):
        where = f"result {i}"
        if not isinstance(result.get("ruleId"), str) or not result["ruleId"]:
            problems.append(f"{where}: ruleId")
        if result.get("level") not in LEVELS:
            problems.append(f"{where}: level")
        text = result.get("message", {}).get("text")
        if not isinstance(text, str) or not text:
            problems.append(f"{where}: message.text")
        physical = result["locations"][0]["physicalLocation"]
        uri = physical["artifactLocation"]["uri"]
        if (
            re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", uri)
            or uri.startswith(("/", "./"))
            or "%" in uri
            or "<external>" in uri
        ):
            problems.append(f"{where}: uri {uri!r} is not repo-relative")
        region = physical["region"]
        line = region.get("startLine")
        if type(line) is not int or line < 1:
            problems.append(f"{where}: startLine")
        if "startColumn" in region:
            column = region["startColumn"]
            if type(column) is not int or column < 1:
                problems.append(f"{where}: startColumn")
    return problems


def _first(sarif: dict, key_path: list[str], value) -> dict:
    sarif = copy.deepcopy(sarif)
    node = sarif["runs"][0]["results"][0]
    for key in key_path[:-1]:
        node = node[key]
    node[key_path[-1]] = value
    return sarif


_LOC = ["locations", 0, "physicalLocation"]


@pytest.fixture(scope="module")
def exported_sarif(slopvac_binary, tmp_path_factory) -> dict:
    out = tmp_path_factory.mktemp("sarif") / "findings.sarif"
    env = {**os.environ, "SLOPVAC_NO_UPDATE_CHECK": "1", "SLOPVAC_OFFLINE": "1"}
    # Run from the repository root so the paths stay relative, as the job does. The
    # fixture is a committed document that the root config lints and that has findings.
    result = subprocess.run(
        [
            str(slopvac_binary),
            "lint",
            ".github/action-fixtures/slop-laden.md",
            "--format",
            "sarif",
            "--out",
            str(out),
        ],
        cwd=ROOT,
        env=env,
        check=False,
        capture_output=True,
    )
    assert result.returncode == 1, result.stderr.decode()
    return json.loads(out.read_text(encoding="utf-8"))


def test_the_exported_sarif_has_the_shape_reviewdog_reads(exported_sarif) -> None:
    assert exported_sarif["runs"][0]["results"], "the fixture must produce findings"
    assert _sarif_problems(exported_sarif) == []


@pytest.mark.parametrize(
    ("path", "value"),
    [
        pytest.param(_LOC + ["artifactLocation", "uri"], "/abs/README.md", id="absolute"),
        pytest.param(_LOC + ["artifactLocation", "uri"], "file:///abs/README.md", id="file-uri"),
        pytest.param(_LOC + ["artifactLocation", "uri"], "./README.md", id="dot-slash"),
        pytest.param(_LOC + ["artifactLocation", "uri"], "my%20doc.md", id="url-encoded"),
        pytest.param(_LOC + ["artifactLocation", "uri"], "<external>/doc.md", id="external"),
        pytest.param(_LOC + ["region", "startLine"], 0, id="start-line-zero"),
        pytest.param(_LOC + ["region", "startLine"], "3", id="start-line-string"),
        pytest.param(_LOC + ["region", "startColumn"], 0, id="start-column-zero"),
        pytest.param(["level"], "fatal", id="bad-level"),
        pytest.param(["ruleId"], "", id="empty-rule-id"),
        pytest.param(["message", "text"], "", id="empty-message"),
    ],
)
def test_a_sarif_that_breaks_the_contract_is_reported(
    exported_sarif, path, value
) -> None:
    broken = _first(exported_sarif, path, value)
    assert _sarif_problems(broken) != []


def test_a_sarif_with_another_version_is_reported(exported_sarif) -> None:
    assert _sarif_problems({**exported_sarif, "version": "2.0.0"}) != []
