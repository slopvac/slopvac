"""Compile every published schema pattern in ECMA-262 Unicode mode.

Published patterns assume ECMA-262 with the ``u`` flag (``docs/contract/policies.md``, section
3.9). Python ``re`` is not ECMA-262, so the compile check runs in Node, which uses the same
regular expression engine as the reference ECMA-262 implementations. Node is pinned in
``mise.toml`` and already runs in CI for the docs site and the Markdown check. The test adds no
dependency. It skips only when ``node`` is not on ``PATH`` and the ``CI`` variable is unset, as
the ``vale`` tests do. In CI a missing ``node`` fails the test.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

_SCHEMAS = Path(__file__).resolve().parents[2] / "schemas/v3"
# Keywords whose values are data, not schemas. A `pattern` property name inside them is not a
# pattern keyword.
_DATA = {"const", "enum", "default", "examples"}
_NAMED = {"properties", "patternProperties", "$defs", "dependentSchemas"}

_NODE_SCRIPT = """
const patterns = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const failed = [];
for (const p of patterns) {
  try { new RegExp(p, 'u'); } catch (e) { failed.push({pattern: p, error: String(e.message)}); }
}
process.stdout.write(JSON.stringify(failed));
"""


def _walk(node: object, pointer: str, found: list[tuple[str, str]]) -> None:
    if isinstance(node, dict):
        for key, member in node.items():
            here = f"{pointer}/{key}"
            if key in _DATA:
                continue
            if key == "pattern" and isinstance(member, str):
                found.append((pointer, member))
            elif key in _NAMED and isinstance(member, dict):
                for name, sub in member.items():
                    if key == "patternProperties":
                        found.append((here, name))
                    _walk(sub, f"{here}/{name}", found)
            else:
                _walk(member, here, found)
    elif isinstance(node, list):
        for i, item in enumerate(node):
            _walk(item, f"{pointer}/{i}", found)


def published_patterns() -> list[tuple[str, str, str]]:
    """Every ``pattern`` value and ``patternProperties`` key in ``schemas/v3``."""
    out: list[tuple[str, str, str]] = []
    for path in sorted(_SCHEMAS.glob("*.schema.json")):
        found: list[tuple[str, str]] = []
        _walk(json.loads(path.read_text(encoding="utf-8")), "", found)
        out.extend((path.name, pointer, pattern) for pointer, pattern in found)
    return out


def uncompilable(patterns: list[str]) -> list[dict]:
    """The patterns that Node refuses to compile with the ``u`` flag, with its error text."""
    done = subprocess.run(
        ["node", "-e", _NODE_SCRIPT],
        input=json.dumps(patterns),
        capture_output=True,
        text=True,
        check=True,
        timeout=60,
    )
    return json.loads(done.stdout)


needs_node = pytest.mark.skipif(
    shutil.which("node") is None and not os.environ.get("CI"),
    reason="node is not on PATH",
)


@pytest.fixture(autouse=True)
def _node_is_present_in_ci() -> None:
    """A missing `node` fails in CI, where a skip would prove nothing."""
    if os.environ.get("CI") and shutil.which("node") is None:
        pytest.fail("node is required in CI to compile the published patterns in Unicode mode")


@needs_node
def test_the_compile_check_reports_a_pattern_that_does_not_compile() -> None:
    # Without this a harness that never fails would pass for every pattern.
    bad = uncompilable(["^a(?![\\s\\S])", "[", "[z-a]"])
    assert [b["pattern"] for b in bad] == ["[", "[z-a]"]


@needs_node
def test_every_published_pattern_compiles_in_unicode_mode() -> None:
    found = published_patterns()
    assert found, "no pattern found: the walk is broken"
    bad = uncompilable([pattern for _, _, pattern in found])
    refused = {b["pattern"]: b["error"] for b in bad}
    report = [
        f"{file}:{pointer}: {refused[pattern]}"
        for file, pointer, pattern in found
        if pattern in refused
    ]
    assert not report, "patterns Node refuses in Unicode mode:\n" + "\n".join(report)


def test_the_walk_finds_the_lock_source_pattern() -> None:
    sources = [
        pointer
        for file, pointer, _ in published_patterns()
        if file == "plugins-lock.schema.json" and pointer.endswith("/Locked/properties/source")
    ]
    assert sources == ["/$defs/Locked/properties/source"]
