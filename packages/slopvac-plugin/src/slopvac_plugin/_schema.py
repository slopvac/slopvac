"""Read the SDK's name and path patterns from the vendored JSON schemas.

The wheel carries ``schemas/plugin-protocol.schema.json`` and
``schemas/common.schema.json`` inside the package. Only a module that runs from
the SDK's own source tree (``packages/slopvac-plugin/src/slopvac_plugin``) may fall
back to the repository's ``schemas/v3``. An installed package without its vendored
copy is a broken install, and the import fails.
"""

from __future__ import annotations

import json
import re
from functools import cache
from importlib import resources
from pathlib import Path
from typing import Any

PROTOCOL = "plugin-protocol.schema.json"
COMMON = "common.schema.json"
# The schemas write "end of string" as this lookahead. Python's $ also matches
# before a trailing newline, so the SDK uses \Z for the same meaning.
END = r"(?![\s\S])"


class SchemaError(RuntimeError):
    """A schema file or pattern the SDK needs is missing or unusable."""


def _source_tree_schemas() -> Path | None:
    """``schemas/v3`` of this repository, when the module runs from its source tree."""
    package = Path(__file__).resolve().parent
    # <repo>/packages/slopvac-plugin/src/slopvac_plugin, the whole suffix
    if package.name != "slopvac_plugin" or package.parent.name != "src":
        return None
    if (
        package.parents[1].name != "slopvac-plugin"
        or package.parents[2].name != "packages"
    ):
        return None
    candidate = package.parents[3] / "schemas" / "v3"
    return candidate if (candidate / PROTOCOL).is_file() else None


def schema_dir() -> Path:
    """The wheel's copy of the schemas, else the repository's, in a source tree."""
    vendored = Path(str(resources.files(__package__) / "schemas"))
    if (vendored / PROTOCOL).is_file():
        return vendored
    source = _source_tree_schemas()
    if source is not None:
        return source
    raise SchemaError(
        f"{PROTOCOL} is missing from the vendored schemas at {vendored}: "
        "the slopvac-plugin install is broken, reinstall it"
    )


@cache
def _document(directory: Path, name: str) -> Any:
    path = directory / name
    try:
        return json.loads(path.read_text("utf-8"))
    except (OSError, ValueError) as exc:
        raise SchemaError(f"cannot read schema {name} at {path}: {exc}") from exc


def _pattern(directory: Path, document: str, pointer: str) -> str:
    node = _document(directory, document)
    try:
        for part in pointer.removeprefix("/").split("/"):
            node = node[part.replace("~1", "/").replace("~0", "~")]
    except (KeyError, IndexError, TypeError) as exc:
        raise SchemaError("no such location in the schema") from exc
    if not isinstance(node, str):
        raise SchemaError("is not a string pattern")
    return node


def load_pattern(
    document: str, pointer: str, *, directory: Path | None = None
) -> re.Pattern[str]:
    """Compile the schema pattern at a JSON pointer for use with ``re.match``.

    The schema's end-of-string lookahead becomes ``\\Z``. A pattern that does not end
    with the lookahead is refused, so a schema change to another ending cannot pass
    unnoticed. ``directory`` is for tests; it defaults to ``schema_dir()``.
    """
    try:
        pattern = _pattern(directory or schema_dir(), document, pointer)
    except SchemaError as exc:
        raise SchemaError(f"{document}#{pointer}: {exc}") from exc
    if not pattern.endswith(END):
        raise SchemaError(
            f"{document}#{pointer}: pattern does not end with {END}: {pattern!r}"
        )
    try:
        return re.compile(pattern.replace(END, r"\Z"))
    except re.error as exc:
        raise SchemaError(f"{document}#{pointer}: {exc}") from exc
