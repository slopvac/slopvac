"""Behaviour of the generated docs site builder, through its public interface."""

from __future__ import annotations

import json
import re
import shutil
import sys
from pathlib import Path

import pytest
import yaml
from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "docs"))

import build_site

GENRES_DIR = ROOT / "crates/slopvac-context/src/builtin/genres"
REAL_SCHEMAS = ROOT / "schemas" / "v3"
# The committed schemas use this author-side prefix; the fixture schemas share it.
OLD_PREFIX = "https://slopvac.github.io/slopvac/schemas/v3/"

DATA_PACK = """\
schema_version = 3
id = "style-pack"
version = "1.2.0"
slopvac_api = 1
kind = "data"
description = "Styles and terms."
license = "MIT"

[provides]
styles = ["styles/*.toml"]
terms = ["terms/a.toml", "terms/b.toml"]

[[requires]]
id = "plug"
version = "^0.1"
"""

PLUGIN_PACK = """\
schema_version = 3
id = "plug"
version = "0.1.0"
slopvac_api = 1
kind = "code"
description = "A | plugin."

[plugin]
command = "python"
args = ["run.py", "--fast"]
env = ["HOME_DIR"]
network = true

[[plugin.hooks]]
kind = "tagger"
id = "fences"

[plugin.features]
required = []
optional = ["document-path"]
"""

CONTRACT = """\
# Contract

## 9. Other

| 1 | `ignored` | Not an error. |

## 10. Error codes

<!-- generated:rpc-codes begin -->
| Code | `slopvac_code` | Meaning |
|---|---|---|
| -32700 | `parse-error` | The body is not valid JSON. |
| -32001 | `protocol-version-unsupported` | No common version. |
<!-- generated:rpc-codes end -->

<!-- generated:host-codes begin -->
| `slopvac_code` | Cause |
|---|---|
| `plugin-timeout` | A timer expired. |
<!-- generated:host-codes end -->

## 11. Security
"""

RULES_YAML = """\
id: demo
title: Demo <category>
description: A demo.
rules:
  - id: no-foo
    name: Avoid foo
    kind: pattern
    severity: error
    message: 'foo found: {match} -- use {replacement}'
    tiers: {strict: enforced, normal: enforced, relaxed: excluded}
    fix: Delete foo.
    examples:
      - bad: Use foo here.
        good: Use bar here.
      - bad: Choose SHA-256, not MD5.
        note: 'preserve: instruction naming a real alternative'
      - bad: Remove this.
        good: ''
        note: The ending is dropped.
    provenance: {source: "Hand written \u2014 draft", url: 'https://example.test/foo'}
"""


FIXTURE_CONFIG = {
    "description": "Demo config.",
    "required": ["schema_version"],
    "properties": {
        "schema_version": {"const": 3, "description": "Version."},
        "mode": {"enum": ["a", "b"], "default": "a", "description": "A | mode."},
        "packs": {"type": "array", "items": {"$ref": "#/$defs/Source"}},
        "tricky": {
            "enum": ["a|b", "c`d"],
            "default": "e`|f",
            "description": "See [docs](nowhere.md) and ![i](x.png) *now* `code`.",
        },
        "levels": {
            "type": "object",
            "propertyNames": {"$ref": "#/$defs/Name"},
            "additionalProperties": {"enum": ["on", "off"]},
        },
    },
    "$defs": {
        "Name": {
            "type": "string",
            "pattern": "^[a-z]+$",
            "description": "A data name; the `x-` rule does not apply.",
        },
        "Source": {
            "type": "object",
            "required": ["id"],
            "properties": {"id": {"type": "string", "description": "Use <id>."}},
        },
    },
}


def _schema(name: str, **extra: object) -> str:
    body = {"$id": OLD_PREFIX + name, "title": f"{name} title", **extra}
    return json.dumps(body, indent=2) + "\n"


@pytest.fixture
def sources(tmp_path: Path) -> dict[str, Path]:
    rules = tmp_path / "rules"
    rules.mkdir()
    (rules / "demo.yml").write_text(RULES_YAML)
    schemas = tmp_path / "schemas"
    schemas.mkdir()
    (schemas / "a.schema.json").write_text(
        _schema(
            "a.schema.json", properties={"b": {"$ref": OLD_PREFIX + "b.schema.json"}}
        )
    )
    (schemas / "b.schema.json").write_text(_schema("b.schema.json"))
    for name in ("plugin-protocol", "plugin-manifest", "common"):
        shutil.copy(REAL_SCHEMAS / f"{name}.schema.json", schemas)
    (schemas / "config.schema.json").write_text(
        _schema("config.schema.json", **FIXTURE_CONFIG)
    )
    packs = tmp_path / "packs"
    for pack_id, body in (("style-pack", DATA_PACK), ("plug", PLUGIN_PACK)):
        (packs / pack_id).mkdir(parents=True)
        (packs / pack_id / "slopvac-plugin.toml").write_text(body)
    contract = tmp_path / "contract"
    contract.mkdir()
    (contract / "plugin-protocol.md").write_text(CONTRACT)
    config = tmp_path / "site.toml"
    config.write_text(
        'site_url = "https://docs.example.org/tools/"\napi_version = "v3"\n'
    )
    return {
        "rules": rules,
        "schemas": schemas,
        "packs": packs,
        "contract": contract,
        "config": config,
        "out": tmp_path / "out",
    }


def _run(sources: dict[str, Path], *extra: str) -> int:
    return build_site.main(
        [
            "--out", str(sources["out"]),
            "--config", str(sources["config"]),
            "--rules-dir", str(sources["rules"]),
            "--schemas-dir", str(sources["schemas"]),
            "--plugins-dir", str(sources["packs"]),
            "--contract-dir", str(sources["contract"]),
            *extra,
        ]
    )  # fmt: skip


def _read(sources: dict[str, Path], path: str) -> str:
    return (sources["out"] / path).read_text()


def test_build_writes_rule_page_with_required_fields(sources: dict[str, Path]) -> None:
    assert _run(sources) == 0
    page = _read(sources, "rules/demo/no-foo.md")
    assert page.startswith("# Avoid foo\n")
    assert "| Severity | error |" in page
    assert "| Profile `relaxed` | `excluded` |" in page
    assert "Delete foo." in page
    assert "<https://example.test/foo>" in page


def test_unicode_dashes_in_catalogue_prose_become_ascii(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "Hand written -- draft" in page
    assert "\u2014" not in page


def test_message_slots_render_as_visible_text_not_html_tags(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "foo found: &lt;match&gt; -- use &lt;replacement&gt;" in page
    assert "<match>" not in page
    assert "Demo &lt;category&gt;" in _read(sources, "rules/demo/index.md")


def test_preserved_example_is_kept_as_written_with_its_reason(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "### Example 1: flagged" in page
    assert "### Example 2: kept as written" in page
    assert "Reason: instruction naming a real alternative" in page
    kept = page.split("### Example 2")[1].split("### Example 3")[0]
    assert "Rewrite" not in kept


def test_empty_rewrite_and_note_are_rendered(sources: dict[str, Path]) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "Rewrite: delete the text." in page
    assert "Note: The ending is dropped." in page


def test_category_and_root_indexes_link_each_page(sources: dict[str, Path]) -> None:
    _run(sources)
    assert "[Avoid foo](no-foo.md): `demo.no-foo`" in _read(
        sources, "rules/demo/index.md"
    )
    assert "(demo/index.md): 1 rules" in _read(sources, "rules/index.md")


def test_protocol_methods_table_lists_requests_and_notifications(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/protocol.md")
    assert (
        "| `initialize` | request | [`InitializeParams`](#initializeparams) "
        "| [`InitializeResult`](#initializeresult) |"
    ) in page
    assert "| `shutdown` | request | none | `null` |" in page
    assert "| `exit` | notification | none | none |" in page


def test_protocol_hook_points_pair_each_kind_with_its_method_and_types(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/protocol.md")
    assert (
        "| `tagger` | `tagger/tag` | [`TagParams`](#tagparams) "
        "| [`TagResult`](#tagresult) | Run the named tagger over each document. |"
    ) in page
    assert "| `binary-extractor` | `binary-extractor/extract` |" in page
    kinds = page.split("## Hook points")[1].split("## Capabilities")[0]
    assert kinds.count("\n| `") == 8


def test_protocol_capabilities_show_declaration_offer_and_reply_fields(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/protocol.md")
    capabilities = page.split("## Capabilities")[1].split("## Errors")[0]
    assert "| `features.optional` |" in capabilities
    assert "| `limits.maxBatchDocuments` | integer | yes | minimum 1 |" in capabilities
    assert "| `capabilities.requiresNetwork` | boolean | yes |" in capabilities


def test_protocol_errors_come_from_the_contract_tables(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    errors = _read(sources, "plugins/protocol.md").split("## Errors")[1]
    errors = errors.split("## Definitions")[0]
    assert "| -32001 | `protocol-version-unsupported` | No common version. |" in errors
    assert "| `plugin-timeout` | A timer expired. |" in errors
    assert "ignored" not in errors


def test_protocol_definitions_follow_the_schema_with_fields(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/protocol.md")
    definition = page.split("### CancelParams")[1].split("###")[0]
    assert "| `id` | integer ([`RequestId`](#requestid)) | yes |" in definition


def test_protocol_without_error_tables_in_the_contract_fails(
    sources: dict[str, Path],
) -> None:
    (sources["contract"] / "plugin-protocol.md").write_text("# Contract\n")
    with pytest.raises(SystemExit, match="Error codes"):
        _run(sources)


def test_manifest_reference_documents_fields_and_the_kind_rule(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/manifest.md")
    top_level = page.split("## Top-level fields", 1)[1].split("\n## ", 1)[0]
    assert "| `id` | string (`PluginId`) | yes |" in top_level
    assert "- When `kind` is `code`, the manifest requires `plugin`." in page
    assert "- For any other kind, the manifest forbids `plugin`." in page
    assert "Keys that start with `x-` are extensions" in page


def test_manifest_reference_has_a_section_for_each_nested_table(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/manifest.md")
    provides = page.split("## `provides`")[1].split("## `")[0]
    assert "| `genres` | array of string (`PackPath`) | no |" in provides
    requires = page.split("## `requires`")[1].split("## `")[0]
    assert "| `version` | string (`VersionReq`) | yes |" in requires
    plugin = page.split("## `plugin`")[1]
    assert "| `command` | string | yes | minimum length 1 |" in plugin
    assert "| `features.required` |" in plugin


def test_pack_page_renders_a_data_pack_from_its_manifest(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/bundled/style-pack/index.md")
    assert page.startswith("# style-pack\n\nStyles and terms.\n")
    assert "| Version | 1.2.0 |" in page
    assert "| Kind | data |" in page
    assert "- styles: `styles/*.toml`" in page
    assert "- terms: `terms/a.toml`, `terms/b.toml`" in page
    assert "- [`plug`](../plug/index.md) `^0.1`" in page
    assert "## Plugin" not in page


def test_pack_page_renders_a_plugin_pack_declaration(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/bundled/plug/index.md")
    assert "Command: `python run.py --fast`" in page
    assert "Network access: yes" in page
    assert "- `HOME_DIR`" in page
    assert "| `tagger` | `fences` |" in page
    assert "Features required: none" in page
    assert "Features optional: `document-path`" in page
    assert page.startswith("# plug\n\nA | plugin.\n")


def test_packs_index_links_every_pack_page(sources: dict[str, Path]) -> None:
    assert _run(sources) == 0
    index = _read(sources, "plugins/bundled/index.md")
    assert "| [plug](plug/index.md) | 0.1.0 | code | A \\| plugin. |" in index
    assert (
        "| [style-pack](style-pack/index.md) | 1.2.0 | data | Styles and terms. |"
        in index
    )


def test_manifest_that_breaks_the_schema_stops_the_build(
    sources: dict[str, Path],
) -> None:
    path = sources["packs"] / "plug" / "slopvac-plugin.toml"
    path.write_text(PLUGIN_PACK.replace('kind = "code"', 'kind = "data"'))
    with pytest.raises(SystemExit, match="plug.*slopvac-plugin.toml.*does not match"):
        _run(sources)


def test_two_packs_with_one_id_stop_the_build(sources: dict[str, Path]) -> None:
    (sources["packs"] / "copy").mkdir()
    (sources["packs"] / "copy" / "slopvac-plugin.toml").write_text(PLUGIN_PACK)
    with pytest.raises(SystemExit, match="plugin id plug also appears"):
        _run(sources)


def test_no_packs_gives_an_index_that_says_so(sources: dict[str, Path]) -> None:
    shutil.rmtree(sources["packs"])
    sources["packs"].mkdir()
    assert _run(sources) == 0
    assert "No plugins are bundled" in _read(sources, "plugins/bundled/index.md")


def test_committed_schemas_contract_and_example_pack_render() -> None:
    sys.path.insert(0, str(ROOT / "scripts" / "docs"))
    import plugin_pages

    schemas = plugin_pages.SchemaSet(REAL_SCHEMAS)
    protocol = plugin_pages.generate_protocol(schemas, ROOT / "docs" / "contract")
    assert "| -32002 | `required-feature-missing` |" in protocol["plugins/protocol.md"]
    packs = plugin_pages.generate_packs(
        schemas, ROOT / "packages" / "slopvac-plugin" / "examples"
    )
    assert "Command: `python plugin.py`" in packs["plugins/bundled/fence-tagger/index.md"]


def test_schemas_are_written_at_their_published_id_path(
    sources: dict[str, Path],
) -> None:
    assert _run(sources) == 0
    for name in ("a", "b", "plugin-protocol"):
        path = f"schemas/v3/{name}.schema.json"
        published = json.loads(_read(sources, path))
        assert published["$id"] == f"https://docs.example.org/tools/{path}"


def test_changing_site_url_rewrites_ids_and_cross_references(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    a = json.loads(_read(sources, "schemas/v3/a.schema.json"))
    assert (
        a["properties"]["b"]["$ref"]
        == "https://docs.example.org/tools/schemas/v3/b.schema.json"
    )
    assert OLD_PREFIX not in _read(sources, "schemas/v3/a.schema.json")


def test_schema_index_links_resolve(sources: dict[str, Path]) -> None:
    _run(sources)
    assert "[a.schema.json](v3/a.schema.json): a.schema.json title" in _read(
        sources, "schemas/index.md"
    )
    assert (sources["out"] / "schemas/v3/a.schema.json").is_file()


def test_schemas_with_mixed_id_prefixes_fail(sources: dict[str, Path]) -> None:
    (sources["schemas"] / "b.schema.json").write_text(
        json.dumps({"$id": "https://other.example/b.schema.json"})
    )
    with pytest.raises(SystemExit, match="different"):
        _run(sources)


def test_site_url_without_trailing_slash_fails(sources: dict[str, Path]) -> None:
    sources["config"].write_text(
        'site_url = "https://example.test/docs"\napi_version = "v3"\n'
    )
    with pytest.raises(SystemExit, match="slash"):
        _run(sources)


def test_rule_url_uses_the_configured_site(sources: dict[str, Path]) -> None:
    site = build_site.load_site(sources["config"])
    assert (
        build_site.rule_url("demo", "no-foo", site)
        == "https://docs.example.org/tools/v3/rules/demo/no-foo/"
    )


def test_check_passes_on_fresh_output_and_changes_nothing(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    before = {p: p.read_bytes() for p in sources["out"].rglob("*") if p.is_file()}
    assert _run(sources, "--check") == 0
    assert {
        p: p.read_bytes() for p in sources["out"].rglob("*") if p.is_file()
    } == before


def test_check_fails_on_edited_missing_and_obsolete_files(
    sources: dict[str, Path], capsys: pytest.CaptureFixture[str]
) -> None:
    _run(sources)
    (sources["out"] / "rules/demo/no-foo.md").write_text("edited\n")
    (sources["out"] / "styles/index.md").unlink()
    (sources["out"] / "old/page.md").parent.mkdir()
    (sources["out"] / "old/page.md").write_text("gone\n")
    assert _run(sources, "--check") == 1
    err = capsys.readouterr().err
    assert "rules/demo/no-foo.md: out of date" in err
    assert "styles/index.md: missing" in err
    assert "old/page.md: obsolete file" in err
    assert (sources["out"] / "old/page.md").exists()


def test_check_fails_when_nothing_was_generated(sources: dict[str, Path]) -> None:
    assert _run(sources, "--check") == 1
    assert not sources["out"].exists()


def test_broken_markdown_and_json_links_are_reported(sources: dict[str, Path]) -> None:
    pages = build_site.build(
        build_site.load_site(sources["config"]), sources["rules"], sources["schemas"]
    )
    pages["a/index.md"] = (
        "[x](missing.md) [y](../schemas/v3/none.json) [ok](../rules/index.md)"
    )
    assert build_site.broken_links(pages) == [
        "a/index.md: broken link missing.md",
        "a/index.md: broken link ../schemas/v3/none.json",
    ]


def test_real_data_builds_clean_with_a_page_per_rule_and_matching_ids() -> None:
    site = build_site.load_site()
    pages = build_site.build(site)
    assert build_site.broken_links(pages) == []
    assert build_site.schema_id_problems(pages, site) == []
    removed = build_site.removed_rules()
    expected = sum(
        1
        for c in build_site.load_categories()
        for r in c.get("rules", [])
        if f"{c['id']}.{r['id']}" not in removed
    )
    rule_pages = [
        p for p in pages if p.startswith("rules/") and not p.endswith("index.md")
    ]
    assert len(rule_pages) == expected


@pytest.mark.parametrize("preset", ["ste", "google"])
def test_every_bundled_style_preset_gets_a_generated_page(
    sources: dict[str, Path], preset: str
) -> None:
    assert _run(sources) == 0
    assert _read(sources, f"styles/{preset}.md").startswith(f"# Style: {preset}\n")


def test_styles_index_links_the_preset_pages_and_is_not_a_placeholder(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    index = _read(sources, "styles/index.md")
    assert "not generated yet" not in index
    assert "(ste.md)" in index
    assert "(google.md)" in index


def test_built_in_terms_page_is_generated_and_linked_from_the_root_index(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    assert _read(sources, "terms/index.md").startswith("# Built-in terms\n")
    assert "(terms/index.md)" in _read(sources, "index.md")


def test_a_failing_reference_example_stops_the_build_and_reports_its_stderr(
    sources: dict[str, Path], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake_cargo = tmp_path / "fake-cargo"
    fake_cargo.write_text(
        "#!/bin/sh\necho 'boom: no such example xyzzy' >&2\nexit 101\n"
    )
    fake_cargo.chmod(0o755)
    build_site.rust_reference.cache_clear()
    monkeypatch.setattr(build_site, "CARGO", str(fake_cargo))
    try:
        with pytest.raises(SystemExit) as failure:
            _run(sources)
    finally:
        build_site.rust_reference.cache_clear()
    message = str(failure.value)
    assert "slopvac-styles" in message
    assert "boom: no such example xyzzy" in message


def test_genre_pages_come_from_the_registry_for_every_builtin_genre(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    ids = [yaml.safe_load(f.read_text())["id"] for f in sorted(GENRES_DIR.glob("*.yaml"))]
    written = {
        p.relative_to(sources["out"]).as_posix()
        for p in (sources["out"] / "genres").glob("*.md")
    }
    assert written == {"genres/index.md", *(f"genres/{i}.md" for i in ids)}
    index = _read(sources, "genres/index.md")
    assert "not generated yet" not in index
    assert all(f"({i}.md)" in index for i in ids)
    assert "Issue comments, discussions, blogs, drafts." in _read(
        sources, "genres/informal.md"
    )


# --- folded allOf branches -------------------------------------------------


def _definition(sources: dict[str, Path], name: str) -> str:
    page = _read(sources, "plugins/protocol.md")
    return page.split(f"### {name}\n")[1].split("\n### ")[0]


def _row(section: str, field: str) -> str:
    (row,) = [ln for ln in section.splitlines() if ln.startswith(f"| `{field}` |")]
    return row


@pytest.mark.parametrize(
    ("definition", "field", "required"),
    [
        ("TagResult", "results[].tags", False),
        ("ExtractResult", "results[].segments", False),
        ("FixResult", "results[].edits", False),
        ("FixParams", "documents[].findings", True),
        ("ScoreParams", "documents[].findings", True),
    ],
)
def test_fields_from_inline_allof_branches_reach_the_tables(
    sources: dict[str, Path], definition: str, field: str, required: bool
) -> None:
    _run(sources)
    row = _row(_definition(sources, definition), field)
    assert row.split(" | ")[2] == ("yes" if required else "no")


def test_inline_branch_fields_sit_beside_the_referenced_branch_fields(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    score = _definition(sources, "ScoreResult")
    assert _row(score, "results[].score").endswith("| number | no | minimum 0 |  |")
    assert "| `results[].document` | string | yes |" in score
    fix = _definition(sources, "FixParams")
    assert "| `documents[].text` | string | yes |" in fix


# --- hostile manifest values -----------------------------------------------

HOSTILE_PACK = """\
schema_version = 3
id = "hostile"
version = "1.0.0"
slopvac_api = 1
kind = "code"
description = "Use <script>alert(1)</script> or [click](javascript:alert(1)) | now"
license = "MIT`<img src=x onerror=alert(1)>"
homepage = "https://example.test/docs"

[provides]
styles = ["a`<script>alert(2)</script>`b.toml", "line\\nbreak|pipe.toml", "`lead`"]

[plugin]
command = "py`<b>x</b>`"
args = ["-c", "`tick`", " pad "]
network = false
hooks = []

[plugin.features]
required = []
optional = []
"""

_CODE_SPAN = re.compile(r"(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)")


def _hostile(sources: dict[str, Path], body: str = HOSTILE_PACK) -> str:
    folder = sources["packs"] / "hostile"
    folder.mkdir()
    (folder / "slopvac-plugin.toml").write_text(body)
    assert _run(sources) == 0
    return _read(sources, "plugins/bundled/hostile/index.md")


def test_no_hostile_manifest_value_leaves_raw_html_outside_code_spans(
    sources: dict[str, Path],
) -> None:
    page = _hostile(sources) + _read(sources, "plugins/bundled/index.md")
    for line in page.splitlines():
        assert "<" not in _CODE_SPAN.sub("", line), line


def test_values_with_backticks_get_a_longer_fence(sources: dict[str, Path]) -> None:
    page = _hostile(sources)
    assert "| License | ``MIT`<img src=x onerror=alert(1)>`` |" in page
    assert "| Homepage | `https://example.test/docs` |" in page
    assert "Command: ``py`<b>x</b>` -c `tick` pad``" in page


def test_newlines_in_paths_become_spaces_and_list_pipes_stay_literal(
    sources: dict[str, Path],
) -> None:
    page = _hostile(sources)
    assert (
        "- styles: ``a`<script>alert(2)</script>`b.toml``, `line break|pipe.toml`, `` `lead` ``"
        in page
    )


def test_markdown_in_a_pack_description_is_escaped(sources: dict[str, Path]) -> None:
    page = _hostile(sources)
    expected = (
        "Use &lt;script&gt;alert\\(1\\)&lt;/script&gt; or "
        "\\[click\\]\\(javascript:alert\\(1\\)\\) | now"
    )
    assert f"\n\n{expected}\n" in page
    cell_text = expected.replace("|", "\\|")
    assert f"| {cell_text} |" in _read(sources, "plugins/bundled/index.md")


def test_a_pipe_in_a_table_value_cannot_split_the_cell(
    sources: dict[str, Path],
) -> None:
    body = HOSTILE_PACK.replace("MIT`<img src=x onerror=alert(1)>", "A|B")
    assert "| License | `A\\|B` |" in _hostile(sources, body)


# --- provides --------------------------------------------------------------


def test_provides_lists_declared_keys_and_ignores_extensions(
    sources: dict[str, Path],
) -> None:
    body = DATA_PACK.replace(
        'terms = ["terms/a.toml", "terms/b.toml"]',
        'terms = ["terms/a.toml"]\nx-count = 3\nx-files = ["secret.toml"]',
    )
    (sources["packs"] / "style-pack" / "slopvac-plugin.toml").write_text(body)
    assert _run(sources) == 0
    page = _read(sources, "plugins/bundled/style-pack/index.md")
    assert "- styles: `styles/*.toml`\n- terms: `terms/a.toml`\n" in page
    assert "x-count" not in page
    assert "secret.toml" not in page


# --- page paths ------------------------------------------------------------


def test_a_pack_named_index_does_not_replace_the_packs_index(
    sources: dict[str, Path],
) -> None:
    folder = sources["packs"] / "idx"
    folder.mkdir()
    body = DATA_PACK.replace('id = "style-pack"', 'id = "index"')
    body = body.replace('[[requires]]\nid = "plug"\nversion = "^0.1"\n', "")
    (folder / "slopvac-plugin.toml").write_text(body)
    assert _run(sources) == 0
    assert _read(sources, "plugins/bundled/index/index.md").startswith("# index\n")
    index = _read(sources, "plugins/bundled/index.md")
    assert index.startswith("# Bundled plugins\n")
    assert "| [index](index/index.md) |" in index


# --- error tables ----------------------------------------------------------


def _with_contract(sources: dict[str, Path], new: str) -> None:
    old = "| -32001 |"
    assert old in CONTRACT
    text = CONTRACT.replace(old, f"{new}\n{old}")
    (sources["contract"] / "plugin-protocol.md").write_text(text)


def test_an_escaped_pipe_stays_inside_its_error_table_cell(
    sources: dict[str, Path],
) -> None:
    _with_contract(sources, "| -32009 | `pipe-case` | Takes a \\| b. |")
    _run(sources)
    errors = _read(sources, "plugins/protocol.md").split("## Errors")[1]
    assert "| -32009 | `pipe-case` | Takes a \\| b. |" in errors


@pytest.mark.parametrize(
    "row",
    [
        "| -32009 | `short-row` |",
        "| -32009 | `split-row` | one | two |",
        "| nine | `bad-code` | Not numeric. |",
    ],
)
def test_a_malformed_error_table_row_stops_the_build(
    sources: dict[str, Path], row: str
) -> None:
    _with_contract(sources, row)
    with pytest.raises(SystemExit, match="malformed"):
        _run(sources)


def _contract(sources: dict[str, Path], text: str) -> str:
    (sources["contract"] / "plugin-protocol.md").write_text(text)
    _run(sources)
    errors = _read(sources, "plugins/protocol.md").split("## Errors")[1]
    return errors.split("## Definitions")[0]


def test_a_row_between_unrelated_markers_is_not_published(
    sources: dict[str, Path],
) -> None:
    extra = (
        "<!-- other begin -->\n| -32020 | `unrelated` | Not generated. |\n"
        "<!-- other end -->\n"
    )
    errors = _contract(
        sources,
        CONTRACT.replace("<!-- generated:host", extra + "<!-- generated:host", 1),
    )
    assert "unrelated" not in errors
    assert "`parse-error`" in errors


def test_a_row_inside_a_code_fence_is_not_published(sources: dict[str, Path]) -> None:
    fence = "```text\n| -32021 | `fenced` | In a fence. |\n```\n"
    for where in ("<!-- generated:rpc-codes end -->", "## 11. Security"):
        errors = _contract(sources, CONTRACT.replace(where, fence + where, 1))
        assert "fenced" not in errors


def test_the_error_heading_does_not_decide_which_rows_are_published(
    sources: dict[str, Path],
) -> None:
    decoy = "## 8. Error codes\n\n| -32022 | `decoy` | Before the real section. |\n\n"
    errors = _contract(
        sources,
        CONTRACT.replace("## 9. Other", decoy + "## 9. Other", 1).replace(
            "## 10. Error codes", "## 10. Failure list", 1
        ),
    )
    assert "decoy" not in errors
    assert "`plugin-timeout`" in errors


@pytest.mark.parametrize("name", ["rpc-codes", "host-codes"])
@pytest.mark.parametrize("damage", ["missing", "twice"])
def test_a_missing_or_repeated_block_stops_the_build(
    sources: dict[str, Path], name: str, damage: str
) -> None:
    begin = f"<!-- generated:{name} begin -->"
    end = f"<!-- generated:{name} end -->"
    if damage == "missing":
        text = CONTRACT.replace(begin, "").replace(end, "")
    else:
        text = CONTRACT.replace("## 11. Security", f"{begin}\n{end}\n## 11. Security")
    (sources["contract"] / "plugin-protocol.md").write_text(text)
    with pytest.raises(SystemExit, match=name):
        _run(sources)


def test_the_real_contract_publishes_exactly_the_rows_of_its_two_blocks() -> None:
    import plugin_pages

    doc = (ROOT / "docs/contract/plugin-protocol.md").read_text()
    expected: dict[str, list[str]] = {}
    for name in ("rpc-codes", "host-codes"):
        block = doc.split(f"<!-- generated:{name} begin -->\n")[1]
        block = block.split(f"<!-- generated:{name} end -->")[0]
        expected[name] = [
            line for line in block.splitlines()[2:] if line.startswith("|")
        ]
    codes, diagnostics = plugin_pages._error_tables(
        ROOT / "docs/contract/plugin-protocol.md"
    )
    assert len(codes) == len(expected["rpc-codes"])
    assert len(diagnostics) == len(expected["host-codes"])
    assert [c[1] for c in codes] == [
        r.split("|")[2].strip().strip("`") for r in expected["rpc-codes"]
    ]
    assert [d[0] for d in diagnostics] == [
        r.split("|")[1].strip().strip("`") for r in expected["host-codes"]
    ]


def test_the_publisher_and_the_rust_gate_use_the_same_block_names() -> None:
    import plugin_pages

    source = (ROOT / "crates/slopvac-plugins/src/error_codes.rs").read_text()
    assert "<!-- generated:{name} begin -->" in source
    assert "<!-- generated:{name} end -->" in source
    cases = re.findall(r'#\[case::\w+\("([a-z-]+)", (?:true|false)\)\]', source)
    assert sorted(cases) == sorted(plugin_pages.ERROR_BLOCKS)
    assert plugin_pages.BLOCK_MARKER == "<!-- generated:{name} {edge} -->"


# --- URLs and link structure -----------------------------------------------


@pytest.mark.parametrize(
    "homepage",
    [
        "ftp://example.test/",
        "javascript:alert(1)",
        "../manifest.md",
        "https://",
        "https:///no-host",
        "https://example.test/a b",
        "https://example.test/a\\tb",
        "https://example.test/`x`",
        "https://example.test/../a.md",
        "https://example.test/a..b",
    ],
)
def test_a_homepage_that_is_not_a_plain_http_url_stops_the_build(
    sources: dict[str, Path], homepage: str
) -> None:
    folder = sources["packs"] / "hostile"
    folder.mkdir()
    body = HOSTILE_PACK.replace("https://example.test/docs", homepage)
    (folder / "slopvac-plugin.toml").write_text(body)
    with pytest.raises(SystemExit, match="homepage"):
        _run(sources)


@pytest.mark.parametrize("homepage", ["https://example.test/docs", "http://a.test"])
def test_a_plain_http_or_https_homepage_is_kept(
    sources: dict[str, Path], homepage: str
) -> None:
    body = HOSTILE_PACK.replace("https://example.test/docs", homepage)
    assert f"| Homepage | `{homepage}` |" in _hostile(sources, body)


def _site(sources: dict[str, Path]) -> Path:
    return sources["out"].parent / "site"


def _starlight_page(sources: dict[str, Path], route: str) -> str:
    return (_site(sources) / "src/content/docs/v3" / route).read_text()


def test_hostile_values_stay_inside_their_code_spans_in_the_site_pages(
    sources: dict[str, Path],
) -> None:
    # A license that opens a link, and a description that closes one.
    body = HOSTILE_PACK.replace("MIT`<img src=x onerror=alert(1)>", "](../")
    folder = sources["packs"] / "hostile"
    folder.mkdir()
    (folder / "slopvac-plugin.toml").write_text(body)
    assert _run(sources, "--starlight", str(_site(sources))) == 0
    page = _starlight_page(sources, "plugins/bundled/hostile/index.md")
    assert "| License | `](../` |\n| Homepage | `https://example.test/docs` |" in page
    parser = MarkdownIt("commonmark", {"html": True}).enable("table")
    tokens = [t for block in parser.parse(page) for t in (block.children or [block])]
    html = [t for t in tokens if t.type.startswith("html")]
    assert not [t for t in html if "<script" in t.content]
    assert any(t.type == "code_inline" and "<script>" in t.content for t in tokens)


def test_link_like_text_in_a_code_span_is_not_checked_or_rewritten(
    sources: dict[str, Path],
) -> None:
    body = DATA_PACK.replace("styles/*.toml", "a](nothing.md)")
    (sources["packs"] / "style-pack" / "slopvac-plugin.toml").write_text(body)
    assert _run(sources, "--starlight", str(_site(sources))) == 0
    page = _starlight_page(sources, "plugins/bundled/style-pack/index.md")
    assert "- styles: `a](nothing.md)`" in page


def test_link_like_text_in_a_rule_example_survives_the_site_rewrite(
    sources: dict[str, Path],
) -> None:
    extra = RULES_YAML.replace("no-foo", "no-link").replace(
        "Use foo here.", "See [the page](protocol.md) here."
    )
    (sources["rules"] / "link.yml").write_text(extra.replace("id: demo", "id: more"))
    assert _run(sources, "--starlight", str(_site(sources))) == 0
    page = _starlight_page(sources, "rules/more/no-link.md")
    assert "```text\nSee [the page](protocol.md) here.\n```" in page


def _json_keys(node: object) -> set[str]:
    """Every key of every ``properties`` object in a schema, by plain recursion."""
    found: set[str] = set()
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "properties" and isinstance(value, dict):
                found |= set(value)
            found |= _json_keys(value)
    elif isinstance(node, list):
        for item in node:
            found |= _json_keys(item)
    return found


def test_config_reference_shows_type_default_requirement_and_description(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "config/index.md")
    assert "| `schema_version` (required) | `3` | none | Version. |" in page
    assert "| `mode` | one of `a`, `b` | `a` | A \\| mode. |" in page
    assert "| `packs[].id` (required) | string | none | Use &lt;id&gt;. |" in page
    assert "(config/index.md)" in _read(sources, "index.md")


def test_config_cells_keep_pipes_backticks_and_link_shaped_text_inert(
    sources: dict[str, Path],
) -> None:
    assert _run(sources) == 0
    page = _read(sources, "config/index.md")
    row = next(r for r in page.splitlines() if r.startswith("| `tricky`"))
    assert row == (
        "| `tricky` | one of `a\\|b`, ``c`d`` | ``e`\\|f`` "
        "| See \\[docs\\]\\(nowhere.md\\) and \\!\\[i\\]\\(x.png\\) "
        "\\*now\\* \\`code\\`. |"
    )
    assert build_site.broken_links({"config/index.md": page, "index.md": ""}) == []


def test_map_keys_get_a_row_with_the_property_names_metadata(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "config/index.md")
    assert (
        "| `levels.<name>` | one of `on`, `off` | none "
        "| A data name; the \\`x-\\` rule does not apply. "
        "Names match the pattern ^\\[a-z\\]+$ |"
    ) in page


@pytest.mark.parametrize(
    "key",
    [
        "rules",
        "categories",
        "genres[].rules",
        "genres[].categories",
        "overrides[].rules",
        "overrides[].categories",
    ],
)
def test_real_config_map_keys_state_that_the_x_convention_does_not_apply(
    key: str,
) -> None:
    page = build_site.build(build_site.load_site())["config/index.md"]
    row = next(r for r in page.splitlines() if r.startswith(f"| `{key}.<name>`"))
    assert "does not apply to them" in row
    assert "qualified" in row


def test_real_config_reference_lists_every_schema_key_with_a_type() -> None:
    schema = json.loads((build_site.SCHEMAS_DIR / "config.schema.json").read_text())
    page = build_site.build(build_site.load_site())["config/index.md"]
    rows = [r for r in page.splitlines() if r.startswith("| `")]
    segments = {
        part.removesuffix("[]") for row in rows for part in row.split("`")[1].split(".")
    }
    assert _json_keys(schema) <= segments
    assert all(r.split(" | ")[1].strip() for r in rows)
    assert "| `network.offline` | boolean | `false` |" in page
    assert (
        "| `guidance.level` | one of `brief`, `standard`, `full` | `standard` |" in page
    )


def test_cli_pages_come_from_the_clap_tree_and_are_covered_by_check(
    sources: dict[str, Path], capsys: pytest.CaptureFixture[str]
) -> None:
    _run(sources)
    assert "(cli/index.md)" in _read(sources, "index.md")
    index = _read(sources, "cli/index.md")
    assert "(lint.md)" in index and "## Exit codes" in index
    assert "`--format`" in _read(sources, "cli/lint.md")
    (sources["out"] / "cli/lint.md").write_text("edited\n")
    assert _run(sources, "--check") == 1
    assert "cli/lint.md: out of date" in capsys.readouterr().err


def test_rule_pages_and_indexes_skip_the_rules_3_0_removed() -> None:
    import tomllib

    shared = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"
    entries = tomllib.loads(shared.read_text(encoding="utf-8"))["rules"]
    removed = [entry["id"] for entry in entries]
    assert removed, "the shared list names the removed rules"
    pages = build_site.generate_rules()
    for key in removed:
        category, rule = key.split(".", 1)
        assert build_site.rule_page_path(category, rule) not in pages, key
        assert rule not in pages[f"rules/{category}/index.md"], key
        assert rule not in pages["rules/index.md"], key
    # A rule next to the removed ones is still listed.
    assert "rules/ste-words/index.md" in pages
    assert "spelling" in pages["rules/ste-words/index.md"]
    assert not [
        p
        for p, text in pages.items()
        if any(k.split(".", 1)[1] in text for k in removed)
    ]


def test_removed_rules_fails_when_the_contract_file_is_missing(
    monkeypatch, tmp_path
) -> None:
    monkeypatch.setattr(build_site, "REMOVED_RULES_FILE", tmp_path / "missing.toml")
    with pytest.raises(FileNotFoundError):
        build_site.removed_rules()
