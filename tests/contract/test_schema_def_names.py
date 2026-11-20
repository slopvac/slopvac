"""A `$defs` name means one thing across schemas/v3.

Two artifacts can each define a `ConfigSource`, but a reader and a generator
resolve a bare name by its meaning, so two definitions with one name and
different content invite a wrong `$ref`. This test fails when two schemas
define the same `$defs` name with different content. An identical copy is
allowed only when it is listed in `INTENTIONAL_COPIES` with a reason.
"""

from __future__ import annotations

import json
from collections import defaultdict
from pathlib import Path

SCHEMA_FILES = sorted(
    (Path(__file__).resolve().parents[2] / "schemas" / "v3").glob("*.schema.json")
)

# def name -> the artifact file names whose identical copies are intended.
INTENTIONAL_COPIES: dict[str, tuple[str, ...]] = {}


def _defs_by_name() -> dict[str, list[tuple[Path, object]]]:
    found: dict[str, list[tuple[Path, object]]] = defaultdict(list)
    for path in SCHEMA_FILES:
        schema = json.loads(path.read_text(encoding="utf-8"))
        for name, body in schema.get("$defs", {}).items():
            found[name].append((path, body))
    return found


def test_def_names_are_unique_across_schemas():
    conflicts = []
    for name, copies in sorted(_defs_by_name().items()):
        if len(copies) < 2:
            continue
        files = sorted(path.name for path, _ in copies)
        bodies = {json.dumps(body, sort_keys=True) for _, body in copies}
        if len(bodies) > 1:
            conflicts.append(f"{name}: {', '.join(files)} differ")
        elif tuple(files) != INTENTIONAL_COPIES.get(name):
            conflicts.append(
                f"{name}: identical copies in {', '.join(files)} not allowed"
            )
    assert not conflicts, "\n".join(conflicts)
