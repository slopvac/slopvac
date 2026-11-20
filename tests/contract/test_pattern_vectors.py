"""Check the shared schema pattern vectors against Python jsonschema.

``schemas/v3/conformance/patterns/vectors.json`` records, for each pattern and input, the
verdict of Python jsonschema and of the Rust jsonschema crate. This test checks the Python
half; ``crates/slopvac-schemas/tests/patterns.rs`` checks the Rust half and that the two
differ only for patterns outside the portable subset.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

VECTORS = (
    Path(__file__).resolve().parents[2] / "schemas/v3/conformance/patterns/vectors.json"
)
_ALL = json.loads(VECTORS.read_text(encoding="utf-8"))["vectors"]


@pytest.mark.parametrize(
    "vector", _ALL, ids=[f"{v['pattern'][:40]}|{v['input']!r}" for v in _ALL]
)
def test_python_gives_the_recorded_verdict(vector: dict) -> None:
    validator = Draft202012Validator({"type": "string", "pattern": vector["pattern"]})
    assert validator.is_valid(vector["input"]) is vector["python"]
