"""The SDK's name and path patterns come from the schemas, not from a copy."""

import json
from pathlib import Path

import pytest
from slopvac_plugin import plugin, validate
from slopvac_plugin._schema import COMMON, PROTOCOL, SchemaError, load_pattern

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
        (plugin.HOOK_ID, "plugin-protocol.schema.json", ("$defs", "HookId")),
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
    ids=["context-tag", "hook-id", "relative-path", "feature-name"],
)
def test_pattern_equals_the_schema_pattern(compiled, document, pointer):
    assert compiled.pattern == schema_pattern(document, *pointer).replace(END, r"\Z")


def scratch_schemas(tmp_path, pattern):
    (tmp_path / COMMON).write_text(
        json.dumps({"$defs": {"Name": {"type": "string", "pattern": pattern}}})
    )
    return tmp_path


def test_a_pattern_without_the_end_lookahead_is_refused_when_loaded(tmp_path):
    directory = scratch_schemas(tmp_path, "^[a-z]+$")
    with pytest.raises(SchemaError, match="does not end with") as caught:
        load_pattern(COMMON, "/$defs/Name/pattern", directory=directory)
    assert COMMON in str(caught.value)
    assert "/$defs/Name/pattern" in str(caught.value)


def test_a_missing_pointer_names_the_file_and_pointer_and_keeps_the_cause(tmp_path):
    directory = scratch_schemas(tmp_path, "^a(?![\\s\\S])")
    with pytest.raises(SchemaError) as caught:
        load_pattern(COMMON, "/$defs/Gone/pattern", directory=directory)
    assert COMMON in str(caught.value)
    assert "/$defs/Gone/pattern" in str(caught.value)
    assert isinstance(caught.value.__cause__, SchemaError)
    assert isinstance(caught.value.__cause__.__cause__, KeyError)


def test_a_schema_that_is_not_json_names_the_file_and_pointer_and_keeps_the_cause(
    tmp_path,
):
    (tmp_path / PROTOCOL).write_text("{not json")
    with pytest.raises(SchemaError) as caught:
        load_pattern(PROTOCOL, "/$defs/X/pattern", directory=tmp_path)
    assert PROTOCOL in str(caught.value)
    assert "/$defs/X/pattern" in str(caught.value)
    assert isinstance(caught.value.__cause__.__cause__, json.JSONDecodeError)


def test_a_missing_schema_file_names_the_file_and_pointer_and_keeps_the_cause(
    tmp_path,
):
    with pytest.raises(SchemaError) as caught:
        load_pattern(PROTOCOL, "/$defs/X/pattern", directory=tmp_path)
    assert PROTOCOL in str(caught.value)
    assert "/$defs/X/pattern" in str(caught.value)
    assert isinstance(caught.value.__cause__.__cause__, OSError)
