"""Read the SDK's name and path patterns from the vendored JSON schemas.

The wheel carries ``schemas/plugin-protocol.schema.json`` and
``schemas/common.schema.json``. A source checkout has no such directory, so the
loader falls back to the repository's ``schemas/v3``.
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


def schema_dir() -> Path:
    """The wheel's copy of the schemas, else the repository's ``schemas/v3``."""
    vendored = Path(str(resources.files("slopvac_plugin") / "schemas"))
    if (vendored / PROTOCOL).is_file():
        return vendored
    for parent in Path(__file__).resolve().parents:
        if (parent / "schemas" / "v3" / PROTOCOL).is_file():
            return parent / "schemas" / "v3"
    raise FileNotFoundError(f"{PROTOCOL} not found")


def translate(pattern: str) -> str:
    """Swap the schema's end-of-string lookahead for ``\\Z``.

    Fails when the pattern does not end with the lookahead, so a schema change to
    another ending cannot pass unnoticed.
    """
    if not pattern.endswith(END):
        raise ValueError(f"schema pattern does not end with {END}: {pattern!r}")
    return pattern.replace(END, r"\Z")


@cache
def _document(name: str) -> Any:
    return json.loads((schema_dir() / name).read_text("utf-8"))


def schema_pattern(document: str, pointer: str) -> str:
    """The raw ``pattern`` string at a JSON pointer such as ``/$defs/X/pattern``."""
    node = _document(document)
    for part in pointer.removeprefix("/").split("/"):
        node = node[part.replace("~1", "/").replace("~0", "~")]
    if not isinstance(node, str):
        raise TypeError(f"{document}#{pointer} is not a string")
    return node


def load_pattern(document: str, pointer: str) -> re.Pattern[str]:
    """Compile the schema pattern at ``pointer`` for use with ``re.match``."""
    return re.compile(translate(schema_pattern(document, pointer)))
