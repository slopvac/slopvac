"""The Trivy gate stays strict; one scoped, expiring ignore covers a known gap."""

from __future__ import annotations

import datetime
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]


def trivy_step() -> dict:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
    steps = workflow["jobs"]["trivy"]["steps"]
    return next(s for s in steps if str(s.get("uses", "")).startswith("aquasecurity/trivy-action@"))


def test_ignore_file_has_one_scoped_expiring_entry() -> None:
    data = yaml.safe_load((ROOT / ".trivyignore.yaml").read_text())
    assert list(data) == ["vulnerabilities"]
    (entry,) = data["vulnerabilities"]
    assert entry["id"] == "CVE-2026-112366"
    assert entry["paths"] == ["site/package-lock.json"]
    assert entry["purls"] == ["pkg:npm/picomatch@4.0.7", "pkg:npm/picomatch@2.3.2"]
    assert entry["statement"].strip()
    assert entry["expired_at"] == datetime.date(2026, 12, 18)


def test_workflow_passes_ignore_file_and_keeps_gate() -> None:
    with_ = trivy_step()["with"]
    assert with_["trivyignores"] == ".trivyignore.yaml"
    assert with_["exit-code"] == "1"
    assert with_["severity"] == "HIGH,CRITICAL"
    assert sorted(with_["scanners"].split(",")) == ["misconfig", "secret", "vuln"]


def test_failed_scan_prints_findings() -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows/security.yml").read_text())
    steps = workflow["jobs"]["trivy"]["steps"]
    printer = next(s for s in steps if s["name"] == "Print Trivy findings")
    assert printer["if"] == "failure()"
    assert "trivy-results.sarif" in printer["run"]
