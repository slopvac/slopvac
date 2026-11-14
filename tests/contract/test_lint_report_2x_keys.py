"""Every 2.x lint JSON key has a stated v3 fate in docs/contract/lint-report.md.

The 2.x report is the pydantic models in ``slopvac.report`` and ``slopvac.model``.
The spec lists each key as ``Model.field`` in code ticks, so a key added to or
removed from 2.x without a row here fails.
"""

from __future__ import annotations

import re
from pathlib import Path

from slopvac.model import (
    AiRegisterSummary,
    CategoryScore,
    DocumentScore,
    Finding,
)
from slopvac.report import CategorySummary, LintReport, RunSummary

SPEC = Path(__file__).resolve().parents[2] / "docs" / "contract" / "lint-report.md"
MODELS = (
    LintReport,
    RunSummary,
    CategorySummary,
    DocumentScore,
    CategoryScore,
    Finding,
    AiRegisterSummary,
)
FATES = {"preserve", "rename", "remove"}


def _spec_rows() -> dict[str, str]:
    rows: dict[str, str] = {}
    for line in SPEC.read_text(encoding="utf-8").splitlines():
        match = re.match(r"\|\s*`(\w+\.\w+)`\s*\|\s*(\w+)\s*\|", line)
        if match:
            rows[match.group(1)] = match.group(2)
    return rows


def test_spec_lists_exactly_the_2x_keys() -> None:
    expected = {f"{m.__name__}.{name}" for m in MODELS for name in m.model_fields}
    assert set(_spec_rows()) == expected


def test_every_row_names_a_known_fate() -> None:
    assert set(_spec_rows().values()) <= FATES
