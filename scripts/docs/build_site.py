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

Rules, schemas and the plugin protocol reference come from real data. Styles,
genres and plugin packs are stub generators that emit a placeholder page until
the owning crates exist.
"""

from __future__ import annotations

import argparse
import html
import json
import posixpath
import re
import shutil
import sys
import tomllib
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

import starlight
import yaml

ROOT = Path(__file__).resolve().parents[2]
RULES_DIR = ROOT / "packages" / "slopvac-lint" / "src" / "slopvac" / "rules"
SCHEMAS_DIR = ROOT / "schemas" / "v3"
DEFAULT_OUT = ROOT / "build" / "site-src"
SITE_CONFIG = ROOT / "docs" / "site.toml"

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


def _one_line(text: object) -> str:
    return " ".join(str(text).split())


def _text(value: object) -> str:
    """Catalogue prose as one Markdown line, with HTML specials escaped."""
    return html.escape(_one_line(value), quote=False)


def _cell(value: object) -> str:
    return _text(value).replace("|", "\\|")


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
    lines += [f"| Profile `{name}` | {_cell(tier)} |" for name, tier in tiers.items()]
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


def generate_rules(rules_dir: Path = RULES_DIR) -> Pages:
    # STUB(slopvac-8l4.19.8): reads the Python reference rule YAML; swap for the slopvac-rules loader.
    pages: Pages = {}
    index = ["# Rules", "", "One page per rule, grouped by category.", ""]
    for category in sorted(load_categories(rules_dir), key=lambda c: c["id"]):
        rules = sorted(category.get("rules", []), key=lambda r: r["id"])
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


def generate_protocol(schemas_dir: Path = SCHEMAS_DIR) -> Pages:
    schema = json.loads((schemas_dir / "plugin-protocol.schema.json").read_text())
    lines = [
        "# Plugin protocol v1",
        "",
        _text(schema["description"]),
        "",
        "## Definitions",
        "",
    ]
    for name, definition in sorted(schema["$defs"].items()):
        kind = definition.get("type") or (
            "union" if {"oneOf", "anyOf"} & definition.keys() else "any"
        )
        description = _text(definition.get("description", "No description."))
        lines.append(f"- `{name}` ({kind}): {description}")
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


def build(
    site: Site,
    rules_dir: Path = RULES_DIR,
    schemas_dir: Path = SCHEMAS_DIR,
) -> Pages:
    pages: Pages = {}
    generated = (
        generate_rules(rules_dir),
        generate_schemas(site, schemas_dir),
        generate_protocol(schemas_dir),
        generate_styles(),
        generate_genres(),
        generate_packs(),
        {"index.md": index_page()},
    )
    for group in generated:
        for path, text in group.items():
            if path in pages:
                raise SystemExit(f"two generators wrote {path}")
            pages[path] = text
    return pages


_LINK = re.compile(r"\]\(([^)#:]+\.(?:md|json))(?:#[^)]*)?\)")


def broken_links(pages: Pages) -> list[str]:
    """Relative links that point at no generated file."""
    problems = []
    for path, text in pages.items():
        if not path.endswith(".md"):
            continue
        for target in _LINK.findall(text):
            resolved = posixpath.normpath(
                posixpath.join(posixpath.dirname(path), target)
            )
            if resolved not in pages:
                problems.append(f"{path}: broken link {target}")
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
    pages = build(site, args.rules_dir, args.schemas_dir)
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
