"""Load the plugin protocol schema and validate messages against it."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .. import framing

PROTOCOL = "plugin-protocol.schema.json"
COMMON = "common.schema.json"


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
            Draft202012Validator, type_checker=checker
        )

    def errors(self, definition: str | None, instance: Any) -> list[str]:
        """Schema violations of ``instance`` against a ``$defs`` entry, or the root."""
        if definition is None:
            schema: dict[str, Any] = self._protocol
        else:
            schema = {"$ref": f"{self._protocol['$id']}#/$defs/{definition}"}
        validator = self._validator_class(schema, registry=self._registry)
        found = sorted(
            validator.iter_errors(instance),
            key=lambda e: list(map(str, e.absolute_path)),
        )
        return [
            f"{'/'.join(map(str, e.absolute_path)) or '<root>'}: {e.message[:200]}"
            for e in found[:3]
        ]
