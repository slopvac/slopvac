"""The SDK's name and path patterns come from the schemas, not from a copy."""

import json
from pathlib import Path

import pytest
from slopvac_plugin import plugin, validate

SCHEMAS = Path(__file__).parents[3] / "schemas" / "v3"
END = r"(?![\s\S])"


def schema_pattern(document, *pointer):
    node = json.loads((SCHEMAS / document).read_text("utf-8"))
    for part in pointer:
        node = node[part]
    return node["pattern"]


@pytest.mark.parametrize(
    ("compiled", "document", "pointer"),
    [
        (validate.KEBAB, "common.schema.json", ("$defs", "ContextTag")),
        (
            validate.RELATIVE_PATH,
            "plugin-protocol.schema.json",
            ("$defs", "RecipeFile", "properties", "path"),
        ),
        (
            plugin.FEATURE_NAME,
            "plugin-protocol.schema.json",
            ("$defs", "FeatureName"),
        ),
    ],
    ids=["kebab", "relative-path", "feature-name"],
)
def test_pattern_equals_the_schema_pattern(compiled, document, pointer):
    assert compiled.pattern == schema_pattern(document, *pointer).replace(END, r"\Z")


def test_a_pattern_without_the_end_lookahead_is_refused():
    from slopvac_plugin._schema import translate

    with pytest.raises(ValueError, match="does not end"):
        translate("^[a-z]+$")
