"""The Trivy gate stays strict; one scoped, expiring ignore covers a known gap."""

from __future__ import annotations

import datetime
import json
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]


def load_workflow() -> dict:
    return yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())


def trivy_step() -> dict:
    workflow = load_workflow()
    steps = workflow["jobs"]["trivy"]["steps"]
    return next(
        s
        for s in steps
        if str(s.get("uses", "")).startswith("aquasecurity/trivy-action@")
    )


def test_ignore_file_has_one_scoped_expiring_entry() -> None:
    data = yaml.safe_load((ROOT / ".trivyignore.yaml").read_text())
    assert list(data) == ["vulnerabilities"]
    (entry,) = data["vulnerabilities"]
    assert entry["id"] == "CVE-2026-112366"
    assert entry["paths"] == ["site/package-lock.json"]
    assert entry["purls"] == ["pkg:npm/picomatch@4.0.7", "pkg:npm/picomatch@2.3.2"]
    assert "slopvac-8l4.14.44" in entry["statement"]
    assert entry["expired_at"] == datetime.date(2026, 12, 18)


def test_workflow_passes_ignore_file_and_keeps_gate() -> None:
    with_ = trivy_step()["with"]
    assert with_["trivyignores"] == ".trivyignore.yaml"
    assert with_["exit-code"] == "1"
    assert with_["severity"] == "HIGH,CRITICAL"
    assert sorted(with_["scanners"].split(",")) == ["misconfig", "secret", "vuln"]


SCRIPT = "scripts/print_trivy_findings.py"
HOSTILE = "::error::injected"


def sarif_with(rule_id: str, text: str, uri: str) -> dict:
    return {
        "runs": [
            {
                "results": [
                    {
                        "ruleId": rule_id,
                        "level": "error",
                        "message": {"text": text},
                        "locations": [
                            {"physicalLocation": {"artifactLocation": {"uri": uri}}}
                        ],
                    },
                    {
                        "ruleId": "AVD-X-1",
                        "level": "warning",
                        "message": {"text": "no place"},
                    },
                ]
            },
            {},
        ]
    }


def run_printer(step: dict, tmp_path: Path, sarif: dict) -> subprocess.CompletedProcess:
    name = Path(step["run"].split()[-1]).name
    (tmp_path / name).write_text(json.dumps(sarif))
    command = step["run"].replace(SCRIPT, str(ROOT / SCRIPT))
    return subprocess.run(
        ["bash", "-euo", "pipefail", "-c", command],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )


JOBS = ["trivy", "trivy-secret"]


def printer_step(job: str) -> dict:
    # Looked up inside the test, so a missing step fails an assertion.
    steps = load_workflow()["jobs"][job]["steps"]
    found = [s for s in steps if s.get("name") == "Print Trivy findings"]
    assert len(found) == 1, f"{job}: expected one Print Trivy findings step"
    return found[0]


@pytest.mark.parametrize("job", JOBS)
def test_failed_scan_prints_findings(job: str, tmp_path: Path) -> None:
    step = printer_step(job)
    assert step["if"] == "failure()"
    done = run_printer(
        step,
        tmp_path,
        sarif_with("CVE-2026-0001", "first line\nsecond line", "a/lock.json"),
    )
    assert done.returncode == 0, done.stderr
    assert done.stdout.splitlines() == [
        "CVE-2026-0001 [error] first line @ a/lock.json",
        "AVD-X-1 [warning] no place @ ?",
    ]


@pytest.mark.parametrize("job", JOBS)
@pytest.mark.parametrize("field", ["rule", "text", "uri"])
def test_printed_findings_start_no_workflow_command(
    job: str, field: str, tmp_path: Path
) -> None:
    step = printer_step(job)
    # A scanned file name or a finding text can start a line of the job log.
    args = {"rule": "CVE-1", "text": "t", "uri": "a"}
    args[field] = HOSTILE
    done = run_printer(
        step, tmp_path, sarif_with(args["rule"], args["text"], args["uri"])
    )
    assert done.returncode == 0, done.stderr
    for line in done.stdout.splitlines():
        assert not line.lstrip().startswith(("::", "##["))
    assert "::" not in done.stdout


def test_trivy_gate_cannot_be_bypassed_by_continue_on_error() -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
    assert "continue-on-error" not in workflow["jobs"]["trivy"]
    assert "continue-on-error" not in trivy_step()


def secret_scan_job() -> dict:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
    return workflow["jobs"]["trivy-secret"]


def test_secret_scan_runs_with_the_same_gate_settings_as_the_full_scan() -> None:
    job = secret_scan_job()
    (scan,) = [
        s
        for s in job["steps"]
        if str(s.get("uses", "")).startswith("aquasecurity/trivy-action@")
    ]
    assert scan["uses"] == trivy_step()["uses"]
    with_ = scan["with"]
    assert with_["scanners"] == "secret"
    assert with_["severity"] == "HIGH,CRITICAL"
    assert with_["exit-code"] == "1"
    assert with_["trivyignores"] == ".trivyignore.yaml"
    assert with_["version"] == trivy_step()["with"]["version"]
    assert "continue-on-error" not in job
    assert "continue-on-error" not in scan
    assert job["permissions"] == {"contents": "read"}


def all_trivy_jobs() -> list[tuple[str, str, dict, list[dict]]]:
    """Every (file, job, trivy step, steps before it) in every workflow."""
    found = []
    for path in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        for name, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
            steps = job.get("steps") or []
            for i, step in enumerate(steps):
                if str(step.get("uses", "")).startswith("aquasecurity/trivy-action@"):
                    found.append((path.name, name, step, steps[:i]))
    return found


def test_every_trivy_step_is_found() -> None:
    assert {(f, j) for f, j, _, _ in all_trivy_jobs()} == {
        ("security.yml", "trivy"),
        ("security.yml", "trivy-secret"),
    }


@pytest.mark.parametrize(
    ("file", "job", "step", "before"),
    all_trivy_jobs(),
    ids=lambda v: v if isinstance(v, str) else "",
)
def test_trivy_reads_config_from_outside_the_checkout(
    file: str, job: str, step: dict, before: list[dict]
) -> None:
    # Trivy reads ./trivy.yaml and ./trivy-secret.yaml from the working
    # directory unless told otherwise, so a pull request could plant either.
    trusted = "${{ runner.temp }}/trivy-trusted/"
    assert step["with"]["trivy-config"] == trusted + "trivy.yaml"
    assert step["env"]["TRIVY_SECRET_CONFIG"] == trusted + "trivy-secret.yaml"
    writers = [s for s in before if s.get("name") == "Write trusted Trivy config"]
    assert len(writers) == 1, f"{file}:{job}"
    run = writers[0]["run"]
    assert "$RUNNER_TEMP/trivy-trusted/trivy.yaml" in run
    assert "$RUNNER_TEMP/trivy-trusted/trivy-secret.yaml" in run
    assert "${{" not in run  # fixed text, no expression


def test_trusted_trivy_config_is_valid_and_keeps_the_gate_strict(
    tmp_path: Path,
) -> None:
    (writer,) = [
        s
        for s in all_trivy_jobs()[0][3]
        if s.get("name") == "Write trusted Trivy config"
    ]
    done = subprocess.run(
        ["bash", "-euo", "pipefail", "-c", writer["run"]],
        env={"RUNNER_TEMP": str(tmp_path), "PATH": "/usr/bin:/bin"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    base = tmp_path / "trivy-trusted"
    config = yaml.safe_load((base / "trivy.yaml").read_text())
    assert not set(config) & {"scan", "exit-code", "ignorefile", "skip-dirs"}
    assert (base / "trivy-secret.yaml").read_text().strip() == "{}"
