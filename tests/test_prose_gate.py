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
from urllib.parse import unquote

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


# ---- the gate step -----------------------------------------------------------------


def _gate_problems(job: dict) -> list[str]:
    """Return the Actions settings and script text that can let a failing status through.

    The shell behaviour itself, including the final exit statement, is owned by the
    subprocess test below, which runs the step's real `run:` text.
    """
    step = _step(job, GATE)
    script = step["run"]
    problems = []
    # The key itself is the problem, whatever its value: an expression can turn it on.
    if "continue-on-error" in step or "continue-on-error" in job:
        problems.append("continue-on-error is present")
    if re.search(r"\|\|\s*(true|:)(\s|$)", script) or re.search(r"\bset\s+\+e\b", script):
        problems.append("the script swallows the status")
    piped = re.search(r"(?<!\|)\|(?!\|)", script) is not None
    if piped and not re.search(r"set\s+-\w*o\s+pipefail|set\s+-\w*\s*-o\s+pipefail", script):
        problems.append("a pipe runs without pipefail")
    return problems


def _mutated(mutate) -> dict:
    job = copy.deepcopy(_prose_job())
    mutate(job, _step(job, GATE))
    return job


def _swallow(job, step):
    step["run"] = step["run"].replace("|| status=$?", "|| true")


def _continue_on(where: str, value):
    def mutate(job, step):
        {"step": step, "job": job}[where]["continue-on-error"] = value

    return mutate


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
        pytest.param(_set_plus_e, id="set-plus-e"),
        pytest.param(_pipe_without_pipefail, id="pipe-without-pipefail"),
    ],
)
def test_a_gate_that_swallows_the_exit_code_is_reported(mutate) -> None:
    assert _gate_problems(_mutated(mutate)) != []


@pytest.mark.parametrize("where", ["step", "job"])
@pytest.mark.parametrize(
    "value",
    [True, "true", "${{ true }}", "${{ inputs.anything }}", False],
    ids=["true", "string-true", "expression-true", "expression-input", "false"],
)
def test_any_continue_on_error_key_is_reported(where, value) -> None:
    assert _gate_problems(_mutated(_continue_on(where, value))) != []


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
FIXTURE = ".github/action-fixtures/slop-laden.md"


def _uri_problem(uri: object) -> str | None:
    if not isinstance(uri, str) or not uri:
        return "uri is empty"
    decoded = unquote(uri)
    if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", decoded):
        return "uri has a scheme or a drive letter"
    if decoded.startswith(("/", "\\", "./")):
        return "uri is absolute or starts with ./"
    if ".." in decoded.split("/"):
        return "uri has a .. segment"
    if "<external>" in decoded:
        return "uri names <external>"
    if uri != FIXTURE:
        return f"uri {uri!r} is not the fixture"
    return None


def _sarif_problems(sarif: dict) -> list[str]:
    problems = []
    if sarif.get("version") != "2.1.0":
        problems.append("version is not 2.1.0")
    runs = sarif.get("runs")
    if not isinstance(runs, list) or not runs:
        return [*problems, "no runs"]
    total = 0
    for r, run in enumerate(runs):
        results = run.get("results") if isinstance(run, dict) else None
        if not isinstance(results, list):
            problems.append(f"run {r}: results is not a list")
            continue
        total += len(results)
        for i, result in enumerate(results):
            problems.extend(_result_problems(f"run {r} result {i}", result))
    if total == 0:
        problems.append("no results in any run")
    return problems


def _result_problems(where: str, result: dict) -> list[str]:
    problems = []
    if not isinstance(result.get("ruleId"), str) or not result["ruleId"]:
        problems.append(f"{where}: ruleId")
    if result.get("level") not in LEVELS:
        problems.append(f"{where}: level")
    text = (result.get("message") or {}).get("text")
    if not isinstance(text, str) or not text:
        problems.append(f"{where}: message.text")
    try:
        physical = result["locations"][0]["physicalLocation"]
        uri = physical["artifactLocation"]["uri"]
        region = physical["region"]
    except (KeyError, IndexError, TypeError):
        return [*problems, f"{where}: no physical location"]
    if (problem := _uri_problem(uri)) is not None:
        problems.append(f"{where}: {problem}")
    line = region.get("startLine")
    if type(line) is not int or line < 1:
        problems.append(f"{where}: startLine")
    if "startColumn" in region:
        column = region["startColumn"]
        if type(column) is not int or column < 1:
            problems.append(f"{where}: startColumn")
    return problems


def _first(sarif: dict, key_path: list, value) -> dict:
    sarif = copy.deepcopy(sarif)
    node = sarif["runs"][0]["results"][0]
    for key in key_path[:-1]:
        node = node[key]
    node[key_path[-1]] = value
    return sarif


_LOC = ["locations", 0, "physicalLocation"]
_URI = _LOC + ["artifactLocation", "uri"]


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
            FIXTURE,
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
        pytest.param(_URI, "/abs/slop-laden.md", id="absolute"),
        pytest.param(_URI, "C:/repo/slop-laden.md", id="drive-letter"),
        pytest.param(_URI, "file:///abs/slop-laden.md", id="file-uri"),
        pytest.param(_URI, "https://example.com/" + FIXTURE, id="other-scheme"),
        pytest.param(_URI, "./" + FIXTURE, id="dot-slash"),
        pytest.param(_URI, "../" + FIXTURE, id="parent-segment"),
        pytest.param(_URI, ".github/../" + FIXTURE, id="inner-parent-segment"),
        pytest.param(_URI, "<external>/slop-laden.md", id="external"),
        pytest.param(_URI, "%3Cexternal%3E/slop-laden.md", id="encoded-external"),
        pytest.param(_URI, "%2Fabs/slop-laden.md", id="encoded-absolute"),
        pytest.param(_URI, "README.md", id="another-file"),
        pytest.param(_URI, "", id="empty-uri"),
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


def _with_run(sarif: dict, run: dict) -> dict:
    return {**sarif, "runs": [*sarif["runs"], run]}


def _absolute_second_run(sarif: dict) -> dict:
    result = copy.deepcopy(sarif["runs"][0]["results"][0])
    result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"] = "/abs/x.md"
    return _with_run(sarif, {"results": [result]})


@pytest.mark.parametrize(
    "variant",
    [
        pytest.param(lambda s: {**s, "runs": [{"results": []}]}, id="zero-results"),
        pytest.param(lambda s: {**s, "runs": []}, id="no-runs"),
        pytest.param(lambda s: _with_run(s, {"results": [{}]}), id="second-run-malformed"),
        pytest.param(lambda s: _with_run(s, {}), id="second-run-without-results"),
        pytest.param(_absolute_second_run, id="second-run-absolute-uri"),
    ],
)
def test_a_sarif_with_a_bad_or_empty_run_is_reported(exported_sarif, variant) -> None:
    assert _sarif_problems(variant(copy.deepcopy(exported_sarif))) != []


def test_a_sarif_with_another_version_is_reported(exported_sarif) -> None:
    assert _sarif_problems({**exported_sarif, "version": "2.0.0"}) != []
