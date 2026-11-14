"""Build the generated reference docs into a source tree for the docs site.

    uv run python scripts/docs/build_site.py          # write build/site-src
    uv run python scripts/docs/build_site.py --check  # build to a temp dir, verify, exit 1 on a problem

Pages are plain Markdown with no site-generator front matter, so any renderer
can consume the tree.
Rules, schemas and the plugin protocol reference come from real data. Styles,
genres and plugin packs are stub generators that emit a placeholder page until
the owning crates exist.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
import tomllib
from collections.abc import Callable
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
RULES_DIR = ROOT / "packages" / "slopvac-lint" / "src" / "slopvac" / "rules"
SCHEMAS_DIR = ROOT / "schemas" / "v3"
DEFAULT_OUT = ROOT / "build" / "site-src"

# The site address lives only in docs/site.toml.
SITE_CONFIG = ROOT / "docs" / "site.toml"


def _load_site_config(path: Path = SITE_CONFIG) -> tuple[str, str]:
    data = tomllib.loads(path.read_text())
    site_url = data["site_url"]
    if not site_url.endswith("/"):
        raise SystemExit(f"{path}: site_url must end with a slash")
    return site_url, data["api_version"]


BASE_URL, API_VERSION = _load_site_config()

Pages = dict[str, str]  # path relative to the site root -> Markdown


def versioned_url(path: str) -> str:
    return f"{BASE_URL}{API_VERSION}/{path}"


def rule_page_path(category: str, rule: str) -> str:
    return f"rules/{category}/{rule}.md"


def rule_url(category: str, rule: str) -> str:
    """Stable public URL of a rule page, as outputs link to it."""
    return versioned_url(rule_page_path(category, rule).removesuffix(".md") + ".html")


def _one_line(text: object) -> str:
    return " ".join(str(text).split())


def _code(text: str) -> str:
    longest = max((len(m) for m in re.findall(r"`+", text)), default=0)
    fence = "`" * (longest + 1)
    return f"{fence} {text} {fence}" if longest else f"`{text}`"


def _rule_page(category: dict, rule: dict) -> str:
    qualified = f"{category['id']}.{rule['id']}"
    tiers = rule.get("tiers") or {
        "strict": "enforced",
        "normal": "enforced",
        "relaxed": "excluded",
    }
    lines = [
        f"# {rule['name']}",
        "",
        f"Rule `{qualified}` in the [{category['title']}](index.md) category.",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Kind | {rule['kind']} |",
        f"| Severity | {rule.get('severity', 'warning')} |",
        f"| Scope | {rule.get('scope', 'prose')} |",
    ]
    lines += [f"| Profile `{name}` | {tier} |" for name, tier in tiers.items()]
    lines += [
        "",
        "## Message",
        "",
        _one_line(rule["message"]).replace("{", "<").replace("}", ">"),
    ]
    if rule.get("fix"):
        lines += ["", "## Fix", "", _one_line(rule["fix"])]
    if rule.get("rationale"):
        lines += ["", "## Rationale", "", _one_line(rule["rationale"])]
    if rule.get("examples"):
        lines += ["", "## Examples", ""]
        for example in rule["examples"]:
            lines.append(f"- Flagged: {_code(_one_line(example['bad']))}")
            if example.get("good"):
                lines.append(f"  Better: {_code(_one_line(example['good']))}")
    if rule.get("exceptions"):
        lines += ["", "## Exceptions", ""]
        lines += [f"- {_one_line(item)}" for item in rule["exceptions"]]
    provenance = rule.get("provenance") or {}
    if provenance.get("source"):
        lines += ["", "## Provenance", "", _one_line(provenance["source"])]
        if provenance.get("url"):
            lines += ["", f"<{provenance['url']}>"]
    return "\n".join(lines) + "\n"


def load_categories(rules_dir: Path = RULES_DIR) -> list[dict]:
    """Every category document; one file may hold several, separated by ``---``."""
    texts = (path.read_text() for path in sorted(rules_dir.glob("*.yml")))
    return [doc for text in texts for doc in yaml.safe_load_all(text) if doc]


def generate_rules(rules_dir: Path = RULES_DIR) -> Pages:
    # STUB(slopvac-8l4.19.8): reads the Python reference rule YAML; swap for the slopvac-rules loader.
    pages: Pages = {}
    index = ["# Rules", "", "One page per rule, grouped by category.", ""]
    for category in sorted(load_categories(rules_dir), key=lambda c: c["id"]):
        rules = sorted(category.get("rules", []), key=lambda r: r["id"])
        cat_lines = [
            f"# {category['title']}",
            "",
            _one_line(category.get("description", "")),
            "",
        ]
        for rule in rules:
            pages[rule_page_path(category["id"], rule["id"])] = _rule_page(
                category, rule
            )
            cat_lines.append(
                f"- [{rule['name']}]({rule['id']}.md): `{category['id']}.{rule['id']}`"
            )
        pages[f"rules/{category['id']}/index.md"] = "\n".join(cat_lines) + "\n"
        index.append(
            f"- [{category['title']}]({category['id']}/index.md): {len(rules)} rules"
        )
    pages["rules/index.md"] = "\n".join(index) + "\n"
    return pages


def generate_schemas(schemas_dir: Path = SCHEMAS_DIR) -> Pages:
    """Publish each committed schema at its ``$id`` path, plus an index."""
    # STUB(slopvac-8l4.19.9): copies hand-written schemas; swap for schemars output from slopvac-model.
    pages: Pages = {}
    index = ["# Schemas", "", f"JSON Schema files for API {API_VERSION}.", ""]
    for path in sorted(schemas_dir.glob("*.schema.json")):
        schema = json.loads(path.read_text())
        expected = f"{BASE_URL}schemas/{API_VERSION}/{path.name}"
        if schema.get("$id") != expected:
            raise SystemExit(
                f"{path.name}: $id is {schema.get('$id')!r}, expected {expected!r}"
            )
        pages[f"schemas/{path.name}"] = path.read_text()
        index.append(
            f"- [{path.name}]({path.name}): {_one_line(schema.get('title', ''))}"
        )
    pages["schemas/index.md"] = "\n".join(index) + "\n"
    return pages


def generate_protocol(schemas_dir: Path = SCHEMAS_DIR) -> Pages:
    schema = json.loads((schemas_dir / "plugin-protocol.schema.json").read_text())
    lines = [
        "# Plugin protocol v1",
        "",
        _one_line(schema["description"]),
        "",
        "## Definitions",
        "",
    ]
    for name, definition in sorted(schema["$defs"].items()):
        kind = definition.get("type") or (
            "union" if {"oneOf", "anyOf"} & definition.keys() else "any"
        )
        lines.append(
            f"- `{name}` ({kind}): {_one_line(definition.get('description', 'No description.'))}"
        )
    return {"plugins/protocol.md": "\n".join(lines) + "\n"}


def _placeholder(path: str, title: str, swap: str) -> Pages:
    return {
        path: f"# {title}\n\nThis reference is not generated yet. Bead {swap} tracks it.\n"
    }


def generate_styles() -> Pages:
    # STUB(slopvac-8l4.19.10): needs style preset resolution (slopvac-8l4.7.1).
    return _placeholder("styles/index.md", "Styles", "slopvac-8l4.19.10")


def generate_genres() -> Pages:
    # STUB(slopvac-8l4.19.11): needs the genre registry (slopvac-8l4.5.7).
    return _placeholder("genres/index.md", "Genres", "slopvac-8l4.19.11")


def generate_packs() -> Pages:
    # STUB(slopvac-8l4.19.12): needs the pack loader (slopvac-8l4.9.1).
    return _placeholder("plugins/packs.md", "Packs", "slopvac-8l4.19.12")


GENERATORS: tuple[Callable[[], Pages], ...] = (
    generate_rules,
    generate_schemas,
    generate_protocol,
    generate_styles,
    generate_genres,
    generate_packs,
)

SECTIONS = (
    ("Rules", "rules/index.md"),
    ("Styles", "styles/index.md"),
    ("Genres", "genres/index.md"),
    ("Plugin protocol", "plugins/protocol.md"),
    ("Packs", "plugins/packs.md"),
    ("Schemas", "schemas/index.md"),
)


def index_page() -> str:
    lines = ["# slopvac reference", ""]
    lines += [f"- [{title}]({path})" for title, path in SECTIONS]
    return "\n".join(lines) + "\n"


def build() -> Pages:
    pages: Pages = {}
    for generator in GENERATORS:
        for path, text in generator().items():
            if path in pages:
                raise SystemExit(f"two generators wrote {path}")
            pages[path] = text
    pages["index.md"] = index_page()
    return pages


_LINK = re.compile(r"\]\(([^)#:]+\.md)(?:#[^)]*)?\)")


def _normalise(path: str) -> str:
    parts: list[str] = []
    for part in path.split("/"):
        if part == "..":
            parts.pop()
        elif part != ".":
            parts.append(part)
    return "/".join(parts)


def broken_links(pages: Pages) -> list[str]:
    """Relative Markdown links that point at no generated page."""
    problems = []
    for path, text in pages.items():
        if not path.endswith(".md"):
            continue
        base = Path(path).parent
        for target in _LINK.findall(text):
            if _normalise((base / target).as_posix()) not in pages:
                problems.append(f"{path}: broken link {target}")
    return problems


def write(pages: Pages, out: Path) -> None:
    if out.exists():
        shutil.rmtree(out)
    for path, text in pages.items():
        target = out / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="build to a temp dir and verify; leave no output",
    )
    args = parser.parse_args(argv)
    pages = build()
    problems = broken_links(pages)
    if args.check:
        with tempfile.TemporaryDirectory() as tmp:
            write(pages, Path(tmp))
    else:
        write(pages, args.out)
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
