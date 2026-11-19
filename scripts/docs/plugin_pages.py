"""Plugin protocol, pack manifest and per-pack reference pages.

The protocol and manifest pages come from the JSON Schemas in ``schemas/<api>/``,
plus the error-code tables of the protocol contract, which the schema cannot
express. A pack page comes from one ``slopvac-plugin.toml`` manifest, checked
against the manifest schema first.

STUB(slopvac-8l4.9.1): no pack loader exists yet, so pack pages list the files
a manifest declares under ``provides``. They do not open those files to list
the rules, styles, genres and terms inside. Swap this for the loader's registry
when it lands.
"""

from __future__ import annotations

import json
import re
import tomllib
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

from mdtext import cell, code, text, untrusted, untrusted_cell

Pages = dict[str, str]

PROTOCOL_SCHEMA = "plugin-protocol.schema.json"
MANIFEST_SCHEMA = "plugin-manifest.schema.json"
MANIFEST_FILE = "slopvac-plugin.toml"
CONTRACT_FILE = "plugin-protocol.md"


# ---------------------------------------------------------------------------
# Schema access


def _qualify(node: object, name: str) -> object:
    """Copy of ``node`` with every local ``$ref`` pointing at file ``name``."""
    if isinstance(node, dict):
        return {
            key: (
                f"{name}{value}"
                if key == "$ref" and isinstance(value, str) and value.startswith("#")
                else _qualify(value, name)
            )
            for key, value in node.items()
        }
    if isinstance(node, list):
        return [_qualify(item, name) for item in node]
    return node


class SchemaSet:
    """The committed schemas, with every ``$ref`` made absolute to a file name."""

    def __init__(self, schemas_dir: Path) -> None:
        self.raw: dict[str, dict] = {}
        self.docs: dict[str, dict] = {}
        for path in sorted(schemas_dir.glob("*.schema.json")):
            doc = json.loads(path.read_text())
            self.raw[path.name] = doc
            self.docs[path.name] = _qualify(doc, path.name)  # type: ignore[assignment]

    def doc(self, name: str) -> dict:
        if name not in self.docs:
            raise SystemExit(f"schema {name} is missing")
        return self.docs[name]

    def defs(self, name: str) -> dict:
        return self.doc(name).get("$defs", {})

    def resolve(self, node: dict) -> tuple[dict, str | None]:
        """Follow ``$ref`` links. Local keys override the target's."""
        name = None
        while "$ref" in node:
            file, _, fragment = node["$ref"].partition("#")
            target: dict = self.doc(file)
            for part in [p for p in fragment.split("/") if p]:
                target = target[part]
            name = fragment.rsplit("/", 1)[-1] or name
            node = _merge(target, {k: v for k, v in node.items() if k != "$ref"})
        return node, name

    def properties(self, node: dict) -> tuple[dict[str, dict], set[str]]:
        """Properties and required names, with every ``allOf`` branch folded in."""
        merged: dict[str, dict] = {}
        required: set[str] = set()
        node, _ = self.resolve(node)
        for part in node.get("allOf", []):
            props, req = self.properties(part)
            for key, value in props.items():
                merged[key] = _merge(merged.get(key, {}), value)
            required |= req
        for key, value in node.get("properties", {}).items():
            merged[key] = _merge(merged.get(key, {}), value)
        required |= set(node.get("required", []))
        return merged, required


def _merge(base: dict, own: object) -> dict:
    """``own`` over ``base``. A bare ``true`` keeps the base unchanged."""
    if not isinstance(own, dict):
        return base
    merged = dict(base)
    for key, value in own.items():
        if key == "properties" and isinstance(base.get("properties"), dict):
            merged[key] = {
                name: _merge(base["properties"].get(name, {}), item)
                for name, item in {**base["properties"], **value}.items()
            }
        else:
            merged[key] = value
    return merged


# ---------------------------------------------------------------------------
# Field tables


@dataclass(frozen=True)
class Row:
    name: str
    type: str
    required: bool
    constraints: str
    description: str


def _constraints(node: dict) -> str:
    parts = []
    for key, label in (
        ("minimum", "minimum"),
        ("maximum", "maximum"),
        ("minLength", "minimum length"),
        ("minItems", "minimum items"),
    ):
        if key in node:
            parts.append(f"{label} {node[key]}")
    if node.get("uniqueItems"):
        parts.append("unique items")
    if "format" in node:
        parts.append(f"format {node['format']}")
    pattern = node.get("pattern")
    if pattern and len(pattern) <= 60:
        parts.append(f"pattern `{pattern}`")
    return "; ".join(parts)


class Renderer:
    def __init__(self, schemas: SchemaSet, link_defs_of: str | None = None) -> None:
        self.schemas = schemas
        self.link_defs_of = link_defs_of

    def type_text(self, node: dict) -> str:
        resolved, name = self.schemas.resolve(node)
        if "enum" in resolved:
            base = "one of " + ", ".join(f"`{json.dumps(v)}`" for v in resolved["enum"])
        elif "const" in resolved:
            base = f"`{json.dumps(resolved['const'])}`"
        elif variants := resolved.get("oneOf") or resolved.get("anyOf"):
            base = " or ".join(self.type_text(v) for v in variants)
        elif resolved.get("type") == "array":
            base = f"array of {self.type_text(resolved.get('items', {}))}"
        elif "type" in resolved:
            base = str(resolved["type"])
        else:
            structured = {"properties", "allOf"} & resolved.keys()
            base = "object" if structured else "any"
        if name and name != "items":
            prefix = f"{self.link_defs_of}#/$defs/" if self.link_defs_of else None
            linked = prefix is not None and node.get("$ref", "").startswith(prefix)
            label = f"[`{name}`](#{name.lower()})" if linked else f"`{name}`"
            return f"{base} ({label})" if not base.startswith("array") else base
        return base

    def rows(self, node: dict, prefix: str = "") -> list[Row]:
        props, required = self.schemas.properties(node)
        rows = []
        for name, prop in props.items():
            resolved, _ = self.schemas.resolve(prop)
            rows.append(
                Row(
                    prefix + name,
                    self.type_text(prop),
                    name in required,
                    _constraints(resolved),
                    resolved.get("description", ""),
                )
            )
            inline = prop
            suffix = ""
            if resolved.get("type") == "array" and "$ref" not in prop:
                inline, suffix = resolved.get("items", {}), "[]"
            if "$ref" not in inline and ("properties" in inline or "allOf" in inline):
                rows += self.rows(inline, f"{prefix}{name}{suffix}.")
        return rows


def _table(rows: list[Row]) -> list[str]:
    lines = [
        "| Field | Type | Required | Constraints | Description |",
        "| --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        lines.append(
            f"| `{row.name}` | {cell(row.type)} | {'yes' if row.required else 'no'} "
            f"| {cell(row.constraints)} | {cell(row.description)} |"
        )
    return lines


def _first_sentence(value: str) -> str:
    one = " ".join(value.split())
    match = re.search(r"\.(\s|$)", one)
    return one[: match.start() + 1] if match else one


# ---------------------------------------------------------------------------
# Protocol reference


@dataclass(frozen=True)
class Method:
    name: str
    request: bool
    params: str | None
    result: str | None
    summary: str


def _call_methods(schemas: SchemaSet) -> list[Method]:
    defs = schemas.defs(PROTOCOL_SCHEMA)
    methods = []
    for entry in defs["Call"]["allOf"]:
        name = entry["if"]["properties"]["method"]["const"]
        then = entry["then"]
        request = "id" in then.get("required", [])
        ref = then.get("properties", {}).get("params", {}).get("$ref")
        params = ref.rsplit("/", 1)[-1] if ref else None
        result = None
        if request:
            candidate = (params or "").removesuffix("Params") + "Result"
            # A request with no result def, such as shutdown, answers null.
            result = candidate if candidate in defs else None
        summary = ""
        if params:
            description = " ".join(defs[params].get("description", "").split())
            summary = _first_sentence(description.removeprefix(f"{name}. "))
        methods.append(Method(name, request, params, result, summary))
    return methods


def _def_link(name: str | None) -> str:
    return f"[`{name}`](#{name.lower()})" if name else "none"


_TABLE_HEADERS = (
    ["Code", "`slopvac_code`", "Meaning"],
    ["`slopvac_code`", "Cause"],
)


def _error_tables(
    contract: Path,
) -> tuple[list[tuple[str, str, str]], list[tuple[str, str]]]:
    """Numeric error codes and host-only diagnostics, from the contract's tables."""
    if not contract.is_file():
        raise SystemExit(f"{contract}: protocol contract not found")
    body = contract.read_text()
    match = re.search(
        r"^## \d+\. Error codes\n(.*?)(?=^## |\Z)", body, re.MULTILINE | re.DOTALL
    )
    if not match:
        raise SystemExit(f"{contract}: no 'Error codes' section")
    codes, diagnostics = [], []
    for line in match[1].splitlines():
        if not line.startswith("|"):
            continue
        if not line.rstrip().endswith("|"):
            raise SystemExit(f"{contract}: malformed error table row: {line}")
        # A pipe escaped as \| belongs to its cell. A split on bare pipes keeps it there.
        cells = [
            c.strip().replace("\\|", "|")
            for c in re.split(r"(?<!\\)\|", line.strip()[1:-1])
        ]
        if all(re.fullmatch(r":?-+:?", c) for c in cells) or cells in _TABLE_HEADERS:
            continue
        if len(cells) == 3 and re.fullmatch(r"-\d+", cells[0]):
            codes.append((cells[0], cells[1].strip("`"), cells[2]))
        elif len(cells) == 2 and re.fullmatch(r"`[a-z][a-z0-9-]*`", cells[0]):
            diagnostics.append((cells[0].strip("`"), cells[1]))
        else:
            raise SystemExit(f"{contract}: malformed error table row: {line}")
    if not codes or not diagnostics:
        raise SystemExit(f"{contract}: error tables not found in 'Error codes'")
    return codes, diagnostics


def generate_protocol(schemas: SchemaSet, contract_dir: Path) -> Pages:
    root = schemas.doc(PROTOCOL_SCHEMA)
    defs = schemas.defs(PROTOCOL_SCHEMA)
    renderer = Renderer(schemas, link_defs_of=PROTOCOL_SCHEMA)
    methods = _call_methods(schemas)
    lines = ["# Plugin protocol v1", "", text(root["description"]), ""]

    lifecycle = [m for m in methods if "/" not in m.name or m.name.startswith("$/")]
    hooks = [m for m in methods if m not in lifecycle]
    lines += [
        "## Lifecycle methods",
        "",
        "| Method | Type | Params | Result | Summary |",
        "| --- | --- | --- | --- | --- |",
    ]
    for m in lifecycle:
        kind = "request" if m.request else "notification"
        result = (
            _def_link(m.result) if m.result else ("`null`" if m.request else "none")
        )
        lines.append(
            f"| `{m.name}` | {kind} | {_def_link(m.params)} | {result} | {cell(m.summary)} |"
        )

    lines += [
        "",
        "## Hook points",
        "",
        text(defs["HookKind"]["description"]),
        "",
        "| Kind | Method | Params | Result | Summary |",
        "| --- | --- | --- | --- | --- |",
    ]
    for m in hooks:
        kind = m.name.split("/", 1)[0]
        lines.append(
            f"| `{kind}` | `{m.name}` | {_def_link(m.params)} "
            f"| {_def_link(m.result)} | {cell(m.summary)} |"
        )
    lines += ["", text(defs["HookId"]["description"])]

    lines += [
        "",
        "## Capabilities",
        "",
        text(defs["FeatureName"]["description"]),
        "",
        "### Declaration before spawn",
        "",
        text(defs["PluginDeclaration"]["description"]),
        "",
        *_table(renderer.rows(defs["PluginDeclaration"])),
        "",
        "### Offer from the host",
        "",
        text(defs["InitializeParams"]["description"]),
        "",
        *_table(renderer.rows(defs["InitializeParams"])),
        "",
        "### Reply from the plugin",
        "",
        text(defs["InitializeResult"]["description"]),
        "",
        *_table(renderer.rows(defs["InitializeResult"])),
    ]

    codes, diagnostics = _error_tables(contract_dir / CONTRACT_FILE)
    lines += [
        "",
        "## Errors",
        "",
        text(defs["ErrorObject"]["description"]),
        "",
        "| Code | `slopvac_code` | Meaning |",
        "| --- | --- | --- |",
    ]
    lines += [f"| {num} | `{slug}` | {cell(meaning)} |" for num, slug, meaning in codes]
    lines += [
        "",
        "The host raises these codes in its own diagnostics. They have no numeric code.",
        "",
        "| `slopvac_code` | Cause |",
        "| --- | --- |",
    ]
    lines += [f"| `{slug}` | {cell(cause)} |" for slug, cause in diagnostics]
    lines += ["", text(defs["ErrorResponse"]["description"])]

    lines += ["", "## Definitions"]
    for name, definition in defs.items():
        lines += ["", f"### {name}", "", text(definition.get("description", ""))]
        rows = renderer.rows(definition)
        if rows:
            lines += ["", *_table(rows)]
        else:
            lines += ["", f"Type: {renderer.type_text(definition)}."]
            constraints = _constraints(definition)
            if constraints:
                lines += ["", f"Constraints: {text(constraints)}."]
    return {"plugins/protocol.md": "\n".join(lines).replace("\n\n\n", "\n\n") + "\n"}


# ---------------------------------------------------------------------------
# Manifest reference


def _kind_rules(schemas: SchemaSet) -> list[str]:
    """Plain-language form of the manifest's ``kind`` / ``plugin`` condition."""
    rules = []
    for entry in schemas.doc(MANIFEST_SCHEMA).get("allOf", []):
        try:
            kind = entry["if"]["properties"]["kind"]["const"]
            needs = entry["then"]["required"]
            forbids = entry["else"]["not"]["required"]
        except KeyError as error:
            raise SystemExit(
                f"{MANIFEST_SCHEMA}: unrecognised condition in allOf (missing {error})"
            ) from error
        rules.append(
            f"- When `kind` is `{kind}`, the manifest requires "
            f"{', '.join(f'`{n}`' for n in needs)}."
        )
        rules.append(
            f"- For any other kind, the manifest forbids "
            f"{', '.join(f'`{n}`' for n in forbids)}."
        )
    return rules


def _extension_note(node: dict) -> str:
    patterns = list(node.get("patternProperties", {}))
    if patterns == ["^x-"] and node.get("additionalProperties") is False:
        return (
            "Keys that start with `x-` are extensions and readers ignore them. "
            "Every other undeclared key is an error."
        )
    return ""


def generate_manifest(schemas: SchemaSet) -> Pages:
    root = schemas.doc(MANIFEST_SCHEMA)
    renderer = Renderer(schemas)
    lines = [
        "# Plugin manifest",
        "",
        text(root["description"]),
        "",
        "## Top-level fields",
        "",
        *_table(renderer.rows(root)),
    ]
    rules = _kind_rules(schemas)
    note = _extension_note(root)
    if rules or note:
        lines += ["", "## Rules", "", *rules]
        if note:
            lines += ["", text(note)]

    props, _ = schemas.properties(root)
    for name, prop in props.items():
        target = prop
        resolved, _ = schemas.resolve(prop)
        if resolved.get("type") == "array":
            target = resolved.get("items", {})
        section, _ = schemas.resolve(target)
        if "$ref" not in target or not ("properties" in section or "allOf" in section):
            continue
        described = resolved.get("description") or section.get("description", "")
        lines += ["", f"## `{name}`", "", text(described)]
        lines += ["", *_table(renderer.rows(target))]
        note = _extension_note(section)
        if note:
            lines += ["", text(note)]
    return {"plugins/manifest.md": "\n".join(lines) + "\n"}


# ---------------------------------------------------------------------------
# Pack pages


def load_manifests(packs_dir: Path) -> dict[str, tuple[Path, dict]]:
    """Every ``<packs_dir>/*/slopvac-plugin.toml``, keyed by pack id."""
    manifests: dict[str, tuple[Path, dict]] = {}
    for path in sorted(packs_dir.glob(f"*/{MANIFEST_FILE}")):
        data = tomllib.loads(path.read_text())
        pack_id = data.get("id")
        if not isinstance(pack_id, str):
            raise SystemExit(f"{path}: manifest has no string id")
        if pack_id in manifests:
            raise SystemExit(
                f"{path}: plugin id {pack_id} also appears in {manifests[pack_id][0]}"
            )
        manifests[pack_id] = (path, data)
    return manifests


def _validate(schemas: SchemaSet, path: Path, manifest: dict) -> None:
    from jsonschema import Draft202012Validator
    from referencing import Registry, Resource
    from referencing.jsonschema import DRAFT202012

    registry = Registry().with_resources(
        [
            (doc["$id"], Resource.from_contents(doc, DRAFT202012))
            for doc in schemas.raw.values()
        ]
    )
    validator = Draft202012Validator(schemas.raw[MANIFEST_SCHEMA], registry=registry)
    errors = sorted(
        validator.iter_errors(manifest), key=lambda e: list(e.absolute_path)
    )
    if errors:
        detail = "; ".join(
            f"{'/'.join(map(str, e.absolute_path)) or '(root)'}: {e.message}"
            for e in errors
        )
        raise SystemExit(f"{path}: manifest does not match {MANIFEST_SCHEMA}: {detail}")
    _check_homepage(path, manifest.get("homepage"))


def _check_homepage(path: Path, homepage: object) -> None:
    """The schema marks homepage as a URI but does not check it. A page shows
    it as text, so it must be a plain http(s) URL with nothing odd in it."""
    if homepage is None:
        return
    parts = urlsplit(str(homepage))
    plain = (
        parts.scheme in ("http", "https")
        and parts.netloc
        and not re.search(r"[\s\x00-\x1f\x7f`]", str(homepage))
        and ".." not in str(homepage)
    )
    if not plain:
        raise SystemExit(
            f"{path}: homepage must be an http or https URL without whitespace, "
            f"backticks or '..': {str(homepage)!r}"
        )


def free(value: object) -> str | None:
    return None if value is None else code(value, in_table=True)


def _pack_page(manifest: dict, known: set[str], artifact_keys: list[str]) -> str:
    """One pack's page. Every manifest value is untrusted, so none is written raw."""
    plugin = manifest.get("plugin")
    # The schema pins version, kind and API to safe forms. License and homepage are free text.
    facts = [
        ("Version", cell(manifest["version"])),
        ("Kind", cell(manifest["kind"])),
        ("Extension API", cell(manifest["slopvac_api"])),
        ("License", free(manifest.get("license"))),
        ("Homepage", free(manifest.get("homepage"))),
    ]
    lines = [f"# {untrusted(manifest['id'])}", ""]
    if manifest.get("description"):
        lines += [untrusted(manifest["description"]), ""]
    lines += ["| Field | Value |", "| --- | --- |"]
    lines += [f"| {label} | {value} |" for label, value in facts if value is not None]

    lines += ["", "## Provides", ""]
    provides = manifest.get("provides", {})
    declared = [(key, provides[key]) for key in artifact_keys if provides.get(key)]
    if declared:
        for kind, paths in declared:
            lines.append(f"- {kind}: " + ", ".join(code(p) for p in paths))
    else:
        lines.append("This plugin declares no data files.")

    requires = manifest.get("requires", [])
    if requires:
        lines += ["", "## Requires", ""]
        for item in requires:
            name = (
                f"[{code(item['id'])}](../{item['id']}/index.md)"
                if item["id"] in known
                else code(item["id"])
            )
            lines.append(f"- {name} {code(item['version'])}")

    if plugin:
        command = " ".join([plugin["command"], *plugin.get("args", [])])
        features = plugin.get("features", {})
        lines += [
            "",
            "## Plugin",
            "",
            f"Command: {code(command)}",
            "",
            f"Network access: {'yes' if plugin.get('network') else 'no'}",
        ]
        if plugin.get("env"):
            lines += ["", "Environment variables the plugin may receive:", ""]
            lines += [f"- {code(name)}" for name in plugin["env"]]
        lines += ["", "### Hooks", ""]
        if plugin.get("hooks"):
            lines += ["| Kind | Id |", "| --- | --- |"]
            lines += [
                f"| {code(h['kind'], in_table=True)} | {code(h['id'], in_table=True)} |"
                for h in plugin["hooks"]
            ]
        else:
            lines.append("The plugin declares no hooks.")
        for label in ("required", "optional"):
            names = features.get(label, [])
            shown = ", ".join(code(n) for n in names) if names else "none"
            lines += ["", f"Features {label}: {shown}"]
    lines += [
        "",
        (
            "See the [manifest reference](../../manifest.md) for every field and the "
            "[protocol reference](../../protocol.md) for hook kinds."
        ),
    ]
    return "\n".join(lines) + "\n"


def generate_packs(schemas: SchemaSet, packs_dir: Path) -> Pages:
    manifests = load_manifests(packs_dir)
    for path, data in manifests.values():
        _validate(schemas, path, data)
    # The artifact kinds a manifest may list, in schema order. Extension keys
    # (x-) are not file lists and never reach the page.
    provides_schema = schemas.defs(MANIFEST_SCHEMA)["Provides"]
    artifact_keys = list(schemas.properties(provides_schema)[0])
    pages: Pages = {}
    known = set(manifests)
    index = ["# Bundled plugins", ""]
    if manifests:
        index += [
            "One page per bundled plugin, generated from its manifest.",
            "",
            "| Plugin | Version | Kind | Description |",
            "| --- | --- | --- | --- |",
        ]
    else:
        index.append("No plugins are bundled with this build.")
    for pack_id, (_, data) in sorted(manifests.items()):
        # A folder per pack keeps the page clear of packs/index.md, whatever the
        # id, and the public route stays /plugins/bundled/<id>/.
        pages[f"plugins/bundled/{pack_id}/index.md"] = _pack_page(
            data, known, artifact_keys
        )
        index.append(
            f"| [{pack_id}]({pack_id}/index.md) | {cell(data['version'])} "
            f"| {cell(data['kind'])} "
            f"| {untrusted_cell(data.get('description', ''))} |"
        )
    pages["plugins/bundled/index.md"] = "\n".join(index) + "\n"
    return pages
