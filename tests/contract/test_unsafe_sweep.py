"""Sweep the published lock ``source`` pattern against every unsafe code point range.

The lock reader refuses any character that ``slopvac_sanitize::is_unsafe`` flags. The expected
ranges here come from the Python SDK's table (``escape_text``), which the same script generates
from the same Unicode data as the Rust table and a test compares the two. The generator for the
schema reads the Rust table, so this test checks the generated file against a second copy and
shares no code with the generator. It reads the committed schema, not a copy of the regex.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator
from referencing import Registry, Resource
from slopvac_plugin.conformance._unsafe_ranges import UNSAFE_RANGES

_SCHEMAS = Path(__file__).resolve().parents[2] / "schemas/v3"
_DIGEST = "sha256:" + "0123456789abcdef" * 4


def _lock_validator() -> Draft202012Validator:
    schemas = [
        json.loads(p.read_text(encoding="utf-8"))
        for p in _SCHEMAS.glob("*.schema.json")
    ]
    registry = Registry().with_resources(
        (s["$id"], Resource.from_contents(s)) for s in schemas
    )
    lock = json.loads((_SCHEMAS / "plugins-lock.schema.json").read_text(encoding="utf-8"))
    return Draft202012Validator(lock, registry=registry)


_VALIDATOR = _lock_validator()


def _document(source: str) -> dict:
    return {
        "schema_version": 3,
        "plugins": [
            {
                "id": "p0000",
                "kind": "data",
                "version": "1.0.0",
                "slopvac_api": 1,
                "source": source,
                "digest": _DIGEST,
            }
        ],
    }


def _samples() -> list[int]:
    """Low bound, high bound and one middle code point of every unsafe range."""
    points: set[int] = set()
    for low, high in UNSAFE_RANGES:
        points.update((low, high, (low + high) // 2))
    return sorted(points)


# One source shape per branch of the pattern: a relative path, and a URL path and authority.
_SHAPES = ["pkg/a{c}b", "https://h/a{c}b", "oci://h{c}x/p", "git+https://h/r{c}@" + "0" * 40]


def test_the_sweep_has_the_expected_ranges() -> None:
    # A guard against an empty or truncated table: the sweep would pass without checking.
    assert len(UNSAFE_RANGES) >= 40
    assert (0x7F, 0x9F) in UNSAFE_RANGES
    assert (0xFEFF, 0xFEFF) in UNSAFE_RANGES


@pytest.mark.parametrize("shape", _SHAPES)
def test_the_lock_source_pattern_refuses_every_unsafe_range(shape: str) -> None:
    accepted = [
        f"U+{code:04X}"
        for code in _samples()
        if _VALIDATOR.is_valid(_document(shape.format(c=chr(code))))
    ]
    assert not accepted, f"the schema accepts unsafe code points: {accepted}"


def test_the_sweep_shapes_accept_a_safe_character() -> None:
    # Without this a shape that is invalid for another reason would pass the sweep.
    for shape in _SHAPES:
        assert _VALIDATOR.is_valid(_document(shape.format(c="x"))), shape
