"""The Trivy gate stays strict; one scoped, expiring ignore covers a known gap."""

from __future__ import annotations

import datetime
import json
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]


def trivy_step() -> dict:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
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


@pytest.mark.skipif(shutil.which("jq") is None, reason="jq is not installed")
def test_failed_scan_prints_findings(tmp_path: Path) -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
    steps = workflow["jobs"]["trivy"]["steps"]
    printer = next(s for s in steps if s["name"] == "Print Trivy findings")
    assert printer["if"] == "failure()"

    sarif = {
        "runs": [
            {
                "results": [
                    {
                        "ruleId": "CVE-2026-0001",
                        "level": "error",
                        "message": {"text": "first line\nsecond line"},
                        "locations": [
                            {
                                "physicalLocation": {
                                    "artifactLocation": {"uri": "a/lock.json"}
                                }
                            }
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
    (tmp_path / "trivy-results.sarif").write_text(json.dumps(sarif))
    done = subprocess.run(
        ["bash", "-euo", "pipefail", "-c", printer["run"]],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    assert done.stdout.splitlines() == [
        "CVE-2026-0001 [error] first line @ a/lock.json",
        "AVD-X-1 [warning] no place @ ?",
    ]


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
