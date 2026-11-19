"""Build the generated reference docs into a source tree for the docs site.

    uv run python scripts/docs/build_site.py          # write build/site-src
    uv run python scripts/docs/build_site.py --check  # compare build/site-src with a fresh build; change nothing

Pages are plain Markdown with no site-generator front matter, so any renderer
can consume the tree. Raw JSON Schema files are written at their identity path,
``schemas/<api>/<name>``, so the file path under the site root plus the site URL
equals the published ``$id``.

The site address comes from ``docs/site.toml``. Committed schemas carry an
author-side ``$id`` prefix. Publishing rewrites that prefix to the configured
site URL, so changing ``site_url`` (for example for a custom domain) needs no
edit to the committed schemas.

Rules, schemas, the plugin protocol and pack manifest references, the pack
pages, styles, terms and genres come from real data. The styles, terms and
genres pages come from each crate's ``reference`` example, which the builder
runs through cargo.
"""

from __future__ import annotations

import argparse
import html
import json
import posixpath
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Sequence
from dataclasses import dataclass
from functools import cache
from pathlib import Path
from urllib.parse import urlsplit

import mdlinks
import plugin_pages
import starlight
import yaml
from mdtext import cell as _cell
from mdtext import code, untrusted, untrusted_cell
from mdtext import one_line as _one_line
from mdtext import text as _text

ROOT = Path(__file__).resolve().parents[2]
RULES_DIR = ROOT / "packages" / "slopvac-lint" / "src" / "slopvac" / "rules"
SCHEMAS_DIR = ROOT / "schemas" / "v3"
REMOVED_RULES_FILE = ROOT / "docs" / "contract" / "compat" / "removed-rules.toml"
DEFAULT_OUT = ROOT / "build" / "site-src"
# Pack manifests to document. The example plugin pack is the only committed one
# until the pack loader and the bundled core pack exist.
PACKS_DIR = ROOT / "packages" / "slopvac-plugin" / "examples"
CONTRACT_DIR = ROOT / "docs" / "contract"
SITE_CONFIG = ROOT / "docs" / "site.toml"
CARGO = "cargo"

Pages = dict[str, str]  # path relative to the site root -> file text


@dataclass(frozen=True)
class Site:
    """The published address, read from docs/site.toml and nowhere else."""

    url: str
    api_version: str

    def versioned_url(self, path: str) -> str:
        return f"{self.url}{self.api_version}/{path}"

    def schema_url(self, name: str) -> str:
        return f"{self.url}{self.schema_path(name)}"

    def schema_path(self, name: str) -> str:
        return f"schemas/{self.api_version}/{name}"


def load_site(path: Path = SITE_CONFIG) -> Site:
    data = tomllib.loads(path.read_text())
    url = data["site_url"]
    if not url.endswith("/"):
        raise SystemExit(f"{path}: site_url must end with a slash")
    return Site(url=url, api_version=data["api_version"])


def rule_page_path(category: str, rule: str) -> str:
    return f"rules/{category}/{rule}.md"


def rule_url(category: str, rule: str, site: Site | None = None) -> str:
    """Stable public URL of a rule page, as outputs link to it."""
    site = site or load_site()
    return site.versioned_url(rule_page_path(category, rule).removesuffix(".md") + "/")


_SLOT = re.compile(r"\{(\w+)\}")


def _message(value: object) -> str:
    """A message template with its ``{slot}`` markers shown as ``<slot>`` text."""
    return _SLOT.sub(r"&lt;\1&gt;", _text(value))


def _fenced(text: str) -> list[str]:
    """A specimen as a fenced block longer than any backtick run it holds."""
    longest = max((len(run) for run in re.findall(r"`+", text)), default=0)
    fence = "`" * max(3, longest + 1)
    return [f"{fence}text", *text.strip("\n").splitlines(), fence]


def _example(number: int, example: dict) -> list[str]:
    note = _one_line(example["note"]) if example.get("note") else ""
    keep = note.startswith("preserve:")
    label = "kept as written" if keep else "flagged"
    lines = ["", f"### Example {number}: {label}", "", *_fenced(example["bad"])]
    good = example.get("good")
    if keep:
        pass
    elif good is None:
        lines += ["", "No rewrite is given."]
    elif not good.strip():
        lines += ["", "Rewrite: delete the text."]
    else:
        lines += ["", "Rewrite:", "", *_fenced(good)]
    if note:
        text = note.removeprefix("preserve:").strip() if keep else note
        lines += [
            "",
            f"{'Reason' if keep else 'Note'}: {html.escape(text, quote=False)}",
        ]
    return lines


def _rule_page(category: dict, rule: dict) -> str:
    qualified = f"{category['id']}.{rule['id']}"
    tiers = rule.get("tiers") or {
        "strict": "enforced",
        "normal": "enforced",
        "relaxed": "excluded",
    }
    lines = [
        f"# {_text(rule['name'])}",
        "",
        f"Rule `{qualified}` in the [{_text(category['title'])}](index.md) category.",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Kind | {_cell(rule['kind'])} |",
        f"| Severity | {_cell(rule.get('severity', 'warning'))} |",
        f"| Scope | {_cell(rule.get('scope', 'prose'))} |",
    ]
    lines += [f"| Profile `{name}` | `{_cell(tier)}` |" for name, tier in tiers.items()]
    lines += ["", "## Message", "", _message(rule["message"])]
    if rule.get("fix"):
        lines += ["", "## Fix", "", _text(rule["fix"])]
    if rule.get("rationale"):
        lines += ["", "## Rationale", "", _text(rule["rationale"])]
    if rule.get("examples"):
        lines += ["", "## Examples"]
        for number, example in enumerate(rule["examples"], start=1):
            lines += _example(number, example)
    if rule.get("exceptions"):
        lines += ["", "## Exceptions", ""]
        lines += [f"- {_text(item)}" for item in rule["exceptions"]]
    provenance = rule.get("provenance") or {}
    if provenance.get("source"):
        lines += ["", "## Provenance", "", _text(provenance["source"])]
        if provenance.get("url"):
            lines += ["", f"<{provenance['url']}>"]
    return "\n".join(lines) + "\n"


def load_categories(rules_dir: Path = RULES_DIR) -> list[dict]:
    """Every category document; one file may hold several, separated by ``---``."""
    texts = (path.read_text() for path in sorted(rules_dir.glob("*.yml")))
    return [doc for text in texts for doc in yaml.safe_load_all(text) if doc]


def removed_rules() -> frozenset[str]:
    """The 2.x rules that 3.0 does not ship, as ``<category>.<rule>``."""
    text = REMOVED_RULES_FILE.read_text(encoding="utf-8")
    return frozenset(entry["id"] for entry in tomllib.loads(text)["rules"])


def generate_rules(rules_dir: Path = RULES_DIR) -> Pages:
    # STUB(slopvac-8l4.19.8): reads the Python reference rule YAML; swap for the slopvac-rules loader.
    pages: Pages = {}
    index = ["# Rules", "", "One page per rule, grouped by category.", ""]
    removed = removed_rules()
    for category in sorted(load_categories(rules_dir), key=lambda c: c["id"]):
        rules = sorted(
            (
                r
                for r in category.get("rules", [])
                if f"{category['id']}.{r['id']}" not in removed
            ),
            key=lambda r: r["id"],
        )
        cat_lines = [
            f"# {_text(category['title'])}",
            "",
            _text(category.get("description", "")),
            "",
        ]
        for rule in rules:
            pages[rule_page_path(category["id"], rule["id"])] = _rule_page(
                category, rule
            )
            cat_lines.append(
                f"- [{_text(rule['name'])}]({rule['id']}.md): `{category['id']}.{rule['id']}`"
            )
        pages[f"rules/{category['id']}/index.md"] = "\n".join(cat_lines) + "\n"
        index.append(
            f"- [{_text(category['title'])}]({category['id']}/index.md): {len(rules)} rules"
        )
    pages["rules/index.md"] = "\n".join(index) + "\n"
    return pages


def generate_schemas(site: Site, schemas_dir: Path = SCHEMAS_DIR) -> Pages:
    """Publish each committed schema at its identity path, plus an index.

    The committed ``$id`` prefix is an author-side placeholder: every schema must
    use the same prefix and end it with its own file name. Publishing replaces
    that prefix with the configured site URL.
    """
    # STUB(slopvac-8l4.19.9): copies hand-written schemas; swap for schemars output from slopvac-model.
    sources = {
        path.name: path.read_text()
        for path in sorted(schemas_dir.glob("*.schema.json"))
    }
    prefixes = set()
    for name, text in sources.items():
        schema_id = json.loads(text).get("$id", "")
        if not schema_id.endswith(f"/{name}"):
            raise SystemExit(f"{name}: $id {schema_id!r} does not end with /{name}")
        prefixes.add(schema_id.removesuffix(name))
    if len(prefixes) > 1:
        raise SystemExit(f"schemas use different $id prefixes: {sorted(prefixes)}")
    pages: Pages = {}
    index = ["# Schemas", "", f"JSON Schema files for API {site.api_version}.", ""]
    for name, text in sources.items():
        for prefix in prefixes:
            text = text.replace(prefix, f"{site.url}{site.schema_path('')}")
        pages[site.schema_path(name)] = text
        title = _text(json.loads(text).get("title", ""))
        index.append(f"- [{name}]({site.api_version}/{name}): {title}")
    pages["schemas/index.md"] = "\n".join(index) + "\n"
    return pages


@cache
def rust_reference(crate: str, example: str = "reference") -> Pages:
    """Pages printed by a crate's reference example, keyed by site path."""
    with tempfile.TemporaryDirectory() as tmp:
        command = [CARGO, "run", "--quiet", "-p", crate, "--example", example]
        result = subprocess.run(
            [*command, "--", tmp], cwd=ROOT, capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            raise SystemExit(f"{crate} reference example failed:\n{result.stderr}")
        return {
            path.relative_to(tmp).as_posix(): path.read_text()
            for path in sorted(Path(tmp).rglob("*.md"))
        }


def generate_styles() -> Pages:
    return rust_reference("slopvac-styles")


def generate_terms() -> Pages:
    return rust_reference("slopvac-terms")


def generate_genres() -> Pages:
    return rust_reference("slopvac-context")


def generate_cli() -> Pages:
    return rust_reference("slopvac", "cli-reference")


def generate_error_codes() -> Pages:
    return rust_reference("slopvac")


class _ConfigSchema:
    """Walks ``config.schema.json`` into one row per key path."""

    def __init__(self, config: dict, common: dict) -> None:
        self.config = config
        self.common = common

    def resolve(self, node: dict) -> dict:
        """The node with a local or ``common.schema.json`` ``$ref`` followed."""
        ref = node.get("$ref")
        if not ref:
            return node
        base, _, pointer = ref.partition("#")
        target = self.common if base == "common.schema.json" else self.config
        for part in pointer.strip("/").split("/"):
            target = target[part]
        own = {k: v for k, v in node.items() if k != "$ref"}
        return {**self.resolve(target), **own}

    def _parts(self, node: dict) -> list[dict]:
        """The node and every schema merged into it by ``allOf`` or ``then``."""
        node = self.resolve(node)
        parts = [node]
        for sub in [
            *node.get("allOf", []),
            *([node["then"]] if "then" in node else []),
        ]:
            parts += self._parts(sub)
        return parts

    def properties(self, node: dict) -> dict[str, dict]:
        found: dict[str, dict] = {}
        for part in self._parts(node):
            found.update(part.get("properties", {}))
        return found

    def required(self, node: dict) -> set[str]:
        return {key for part in self._parts(node) for key in part.get("required", [])}

    @staticmethod
    def literal(value: object) -> str:
        return code(
            value if isinstance(value, str) else json.dumps(value), in_table=True
        )

    def kind(self, node: dict) -> str:
        node = self.resolve(node)
        if "enum" in node:
            return "one of " + ", ".join(self.literal(v) for v in node["enum"])
        if "const" in node:
            return self.literal(node["const"])
        if "oneOf" in node:
            return " or ".join(self.kind(n) for n in node["oneOf"])
        if "then" in node:
            return f"{self.kind(node['else'])} or table"
        kind = node.get("type")
        if kind == "array":
            return f"array of {self.kind(node.get('items', {}))}"
        if kind == "object" and isinstance(node.get("additionalProperties"), dict):
            return f"table of {self.kind(node['additionalProperties'])}"
        if kind in (None, "object"):
            return "table"
        return kind if isinstance(kind, str) else " or ".join(kind)

    def rows(self, node: dict, prefix: str = "") -> list[tuple[str, str, str, str]]:
        found = []
        required = self.required(node)
        for key, child in self.properties(node).items():
            path = prefix + key
            resolved = self.resolve(child)
            default = resolved.get("default")
            found.append(
                (
                    path + (" (required)" if key in required else ""),
                    self.kind(child),
                    "none" if default is None else self.literal(default),
                    resolved.get("description", ""),
                )
            )
            found += self.children(resolved, path)
        return found

    def map_rows(
        self, node: dict, values: dict, path: str
    ) -> list[tuple[str, str, str, str]]:
        """A row for the user-chosen key, then rows for the keys of each value.

        The key row carries the ``propertyNames`` metadata: which names are
        allowed and that ``x-`` has no special meaning for them.
        """
        names = self.resolve(node.get("propertyNames", {}))
        pattern = names.get("pattern")
        description = names.get("description", "A name chosen by the user.")
        if pattern:
            description += f" Names match the pattern {pattern}"
        key_row = (f"{path}.<name>", self.kind(values), "none", description)
        return [key_row, *self.rows(values, path + ".<name>.")]

    def children(self, node: dict, path: str) -> list[tuple[str, str, str, str]]:
        if node.get("type") == "array" and "items" in node:
            return self.rows(node["items"], path + "[].")
        extra = node.get("additionalProperties")
        if isinstance(extra, dict):
            return self.map_rows(node, extra, path)
        return self.rows(node, path + ".")


def generate_config(schemas_dir: Path = SCHEMAS_DIR) -> Pages:
    config = json.loads((schemas_dir / "config.schema.json").read_text())
    common = json.loads((schemas_dir / "common.schema.json").read_text())
    lines = [
        "# Configuration reference",
        "",
        untrusted(config.get("description", "")),
        "",
        "This page writes a key inside an array of tables as `name[].key`.",
        "It writes a key inside a table of user-chosen names as `table.<name>.key`.",
        "",
        "| Key | Type | Default | Description |",
        "| --- | --- | --- | --- |",
    ]
    for path, kind, default, description in _ConfigSchema(config, common).rows(config):
        key, _, note = path.partition(" ")
        label = code(key, in_table=True) + (" " + note if note else "")
        lines.append(
            f"| {label} | {kind} | {default} | {untrusted_cell(description)} |"
        )
    return {"config/index.md": "\n".join(lines) + "\n"}


SECTIONS = (
    ("Rules", "rules/index.md"),
    ("Styles", "styles/index.md"),
    ("Genres", "genres/index.md"),
    ("Built-in terms", "terms/index.md"),
    ("Plugin protocol", "plugins/protocol.md"),
    ("Pack manifest", "plugins/manifest.md"),
    ("Packs", "plugins/packs/index.md"),
    ("Command line", "cli/index.md"),
    ("Error codes", "reference/error-codes.md"),
    ("Configuration", "config/index.md"),
    ("Schemas", "schemas/index.md"),
)


def index_page() -> str:
    lines = ["# slopvac reference", ""]
    lines += [f"- [{title}]({path})" for title, path in SECTIONS]
    return "\n".join(lines) + "\n"


def build(
    site: Site,
    rules_dir: Path = RULES_DIR,
    schemas_dir: Path = SCHEMAS_DIR,
    packs_dir: Path = PACKS_DIR,
    contract_dir: Path = CONTRACT_DIR,
) -> Pages:
    pages: Pages = {}
    schemas = plugin_pages.SchemaSet(schemas_dir)
    generated = (
        generate_rules(rules_dir),
        generate_schemas(site, schemas_dir),
        plugin_pages.generate_protocol(schemas, contract_dir),
        plugin_pages.generate_manifest(schemas),
        generate_styles(),
        generate_terms(),
        generate_genres(),
        plugin_pages.generate_packs(schemas, packs_dir),
        generate_cli(),
        generate_error_codes(),
        generate_config(schemas_dir),
        {"index.md": index_page()},
    )
    for group in generated:
        for path, text in group.items():
            if path in pages:
                raise SystemExit(f"two generators wrote {path}")
            pages[path] = text
    return pages


def broken_links(pages: Pages) -> list[str]:
    """Relative links that point at no generated file."""
    problems = []
    for path, text in pages.items():
        if not path.endswith(".md"):
            continue
        for link in mdlinks.find_links(text):
            resolved = posixpath.normpath(
                posixpath.join(posixpath.dirname(path), link.target)
            )
            if resolved not in pages:
                problems.append(f"{path}: broken link {link.target}")
    return problems


def schema_id_problems(pages: Pages, site: Site) -> list[str]:
    """Published schema files whose ``$id`` is not their site URL."""
    problems = []
    for path, text in pages.items():
        if path.endswith(".schema.json"):
            schema_id = json.loads(text).get("$id")
            if schema_id != site.url + path:
                problems.append(
                    f"{path}: $id is {schema_id!r}, expected {site.url + path!r}"
                )
    return problems


def stale_problems(pages: Pages, out: Path) -> list[str]:
    """Differences between a fresh build and the files under ``out``. Reads only."""
    if not out.is_dir():
        return [f"{out}: no generated output; run `just docs`"]
    problems = []
    existing = {p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file()}
    for path in sorted(existing - pages.keys()):
        problems.append(f"{path}: obsolete file")
    for path, text in sorted(pages.items()):
        if path not in existing:
            problems.append(f"{path}: missing")
        elif (out / path).read_text() != text:
            problems.append(f"{path}: out of date")
    return problems


def write(pages: Pages, out: Path) -> None:
    if out.exists():
        shutil.rmtree(out)
    for path, text in pages.items():
        target = out / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--config", type=Path, default=SITE_CONFIG)
    parser.add_argument("--rules-dir", type=Path, default=RULES_DIR)
    parser.add_argument("--schemas-dir", type=Path, default=SCHEMAS_DIR)
    parser.add_argument("--packs-dir", type=Path, default=PACKS_DIR)
    parser.add_argument("--contract-dir", type=Path, default=CONTRACT_DIR)
    parser.add_argument(
        "--check",
        action="store_true",
        help="compare --out with a fresh build and change nothing; exit 1 on a difference",
    )
    parser.add_argument(
        "--starlight",
        type=Path,
        metavar="SITE_DIR",
        help="also adapt the pages for the Starlight project in SITE_DIR",
    )
    args = parser.parse_args(argv)
    site = load_site(args.config)
    pages = build(
        site, args.rules_dir, args.schemas_dir, args.packs_dir, args.contract_dir
    )
    problems = broken_links(pages) + schema_id_problems(pages, site)
    if args.check:
        problems += stale_problems(pages, args.out)
    elif not problems:
        write(pages, args.out)
        if args.starlight:
            parts = urlsplit(site.url)
            starlight.write_site(
                pages,
                args.starlight,
                site.api_version,
                site_url=f"{parts.scheme}://{parts.netloc}",
                base=parts.path.rstrip("/") or "/",
            )
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
