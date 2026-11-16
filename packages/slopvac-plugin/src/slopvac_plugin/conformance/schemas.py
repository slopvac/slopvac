"""Load the plugin protocol schema and validate messages against it."""

from __future__ import annotations

import json
from collections.abc import Iterator
from itertools import islice
from pathlib import Path
from typing import Any

from .. import framing

PROTOCOL = "plugin-protocol.schema.json"
COMMON = "common.schema.json"
MAX_REPORTED = 3
# Longest array the suite checks for uniqueness, and longest hook list it accepts.
# The protocol sets no count (the Rust host has none either); this bounds suite work
# against threat model section 5.5 (TM-2). Real plugins declare a handful of hooks.
MAX_UNIQUE_ITEMS = 1024


def canonical(value: Any) -> str:
    """A string that is equal for equal JSON values, so uniqueness is a set lookup."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def bounded_unique_items(
    validator: Any, enabled: bool, instance: Any, _schema: Any
) -> Iterator[Any]:
    """``uniqueItems`` in linear time, for at most ``MAX_UNIQUE_ITEMS`` items.

    The stock keyword compares all pairs when an array mixes types, so a reply of a
    few tens of thousands of names and one number burns CPU outside any deadline.
    """
    from jsonschema import ValidationError

    if not enabled or not validator.is_type(instance, "array"):
        return
    if len(instance) > MAX_UNIQUE_ITEMS:
        yield ValidationError(
            f"{len(instance)} items is more than the limit of {MAX_UNIQUE_ITEMS}"
        )
        return
    seen: set[str] = set()
    for item in instance:
        key = canonical(item)
        if key in seen:
            yield ValidationError(f"{item!r} is a duplicate: items must be unique")
            return
        seen.add(key)


class MissingDependency(RuntimeError):
    """The conformance suite needs the ``jsonschema`` package."""


def schema_dir() -> Path:
    """The wheel's copy of the schemas, else the repository's ``schemas/v3``."""
    here = Path(__file__).resolve()
    vendored = here.parents[1] / "schemas"
    if (vendored / PROTOCOL).is_file():
        return vendored
    for parent in here.parents:
        if (parent / "schemas" / "v3" / PROTOCOL).is_file():
            return parent / "schemas" / "v3"
    raise FileNotFoundError("plugin-protocol.schema.json not found")


class Schemas:
    def __init__(self) -> None:
        try:
            from jsonschema import Draft202012Validator, validators
            from referencing import Registry, Resource
        except ImportError as exc:  # pragma: no cover - exercised by the CLI only
            raise MissingDependency(
                "install slopvac-plugin[conformance] to get jsonschema"
            ) from exc
        directory = schema_dir()
        self._protocol: dict[str, Any] = json.loads(
            (directory / PROTOCOL).read_text("utf-8")
        )
        common = json.loads((directory / COMMON).read_text("utf-8"))
        self._registry = Registry().with_resources(
            [(s["$id"], Resource.from_contents(s)) for s in (self._protocol, common)]
        )
        # Draft 2020-12 accepts 1.0 as an integer. Protocol v1 does not (section 2.2),
        # and the strict parser keeps 1.0 a float, so only an int token is an integer.
        # The parser marks a token beyond 2^53-1 as UnsafeInt, a subclass of int, so the
        # check is on the exact type and the safe range.
        checker = Draft202012Validator.TYPE_CHECKER.redefine(
            "integer",
            lambda _checker, value: (
                type(value) is int and abs(value) <= framing.MAX_SAFE_INT
            ),
        )
        self._validator_class = validators.extend(
            Draft202012Validator,
            validators={"uniqueItems": bounded_unique_items},
            type_checker=checker,
        )

    def errors(self, definition: str | None, instance: Any) -> list[str]:
        """Schema violations of ``instance`` against a ``$defs`` entry, or the root."""
        if definition is None:
            schema: dict[str, Any] = self._protocol
        else:
            schema = {"$ref": f"{self._protocol['$id']}#/$defs/{definition}"}
        validator = self._validator_class(schema, registry=self._registry)
        # Lazily take a few: a reply with hundreds of thousands of invalid entries must
        # not be exhausted, sorted or held in memory (threat model section 5.5).
        found = islice(validator.iter_errors(instance), MAX_REPORTED)
        return [
            f"{'/'.join(map(str, e.absolute_path)) or '<root>'}: {e.message[:200]}"
            for e in found
        ]
