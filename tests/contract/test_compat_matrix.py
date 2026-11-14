"""The committed 2.x compatibility matrix matches its generator and is complete."""

from __future__ import annotations

import importlib.util
import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "compat_matrix", ROOT / "scripts" / "contract" / "compat_matrix.py"
)
compat = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(compat)

OWNER = re.compile(r"^slopvac-8l4(\.\d+)+$")


@pytest.fixture(scope="module")
def rows() -> list[dict]:
    return compat.build()


def test_committed_files_equal_regenerated_output(rows):
    assert compat.MATRIX_JSON.read_text(encoding="utf-8") == compat.render_json(rows)
    assert compat.MATRIX_MD.read_text(encoding="utf-8") == compat.render_md(rows)


def test_every_row_has_disposition_and_owner(rows):
    bad = []
    for row in rows:
        if row["disposition"] not in compat.VALID_DISPOSITIONS:
            bad.append(f"{row['id']}: disposition {row['disposition']!r}")
        if not OWNER.match(row["owner"]):
            bad.append(f"{row['id']}: owner {row['owner']!r}")
        if not row["note"]:
            bad.append(f"{row['id']}: empty note")
    assert not bad, bad


def test_no_orphan_disposition_keys():
    source_ids = {r["id"] for r in compat.source_rows()}
    orphans = [
        key
        for key in compat.load_dispositions()
        if key not in source_ids and not key.startswith(compat.BEHAVIOR_PREFIX)
    ]
    assert not orphans


def test_preserve_rows_have_pending_test_column(rows):
    assert all(r["test"] == "pending" for r in rows if r["disposition"] == "preserve")
    assert all(r["test"] == "n/a" for r in rows if r["disposition"] != "preserve")
