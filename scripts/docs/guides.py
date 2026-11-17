"""Hand-written guides: ``docs/guides/<slug>.md`` becomes ``guides/<slug>.md`` in the site tree.

A guide takes its title and description from YAML front matter (``title``,
``description``) or, when those are absent, from its first ``# `` heading and
the paragraph under it. The page keeps the body and gets a single leading
``# Title`` line, like every generated page. A guide may link to any other page
of the site tree with a relative ``.md`` link; the shared link check reads those
links. Without any guide, no guide page and no index exist.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

Pages = dict[str, str]

_SLUG = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
_FRONT_MATTER = re.compile(r"\A---\n(.*?\n)?---\n", re.DOTALL)
_H1 = re.compile(r"^# (.+?)[ \t]*$", re.MULTILINE)


def _split(path: Path) -> tuple[str, str, str]:
    """Title, description and body (without its first H1) of one guide."""
    text = path.read_text().replace("\r\n", "\n")
    meta: object = {}
    if match := _FRONT_MATTER.match(text):
        meta = yaml.safe_load(match[1] or "") or {}
        text = text[match.end() :]
    if not isinstance(meta, dict):
        raise SystemExit(f"{path}: front matter must be a mapping")
    heading = _H1.search(text)
    title = meta.get("title") or (heading[1] if heading else None)
    if not isinstance(title, str) or not title.strip():
        raise SystemExit(f"{path}: needs a title in front matter or a '# ' heading")
    body = text
    if heading:
        body = text[: heading.start()] + text[heading.end() :]
    body = body.strip("\n")
    description = meta.get("description")
    if description is None:
        first = re.split(r"\n\s*\n", body, maxsplit=1)[0] if body else ""
        description = (
            ""
            if first.lstrip().startswith(("#", "```", "~~~", "|", "-", "*"))
            else first
        )
    if not isinstance(description, str):
        raise SystemExit(f"{path}: description must be a string")
    return title.strip(), " ".join(description.split()), body


def generate_guides(guides_dir: Path) -> Pages:
    """Guide pages plus a ``guides/index.md``; nothing if there is no guide."""
    files = sorted(guides_dir.glob("*.md")) if guides_dir.is_dir() else []
    pages: Pages = {}
    entries = []
    for path in files:
        slug = path.stem
        if slug == "index" or not _SLUG.fullmatch(slug):
            raise SystemExit(
                f"{path}: guide file names are lowercase words joined by '-' and not 'index'"
            )
        title, description, body = _split(path)
        pages[f"guides/{slug}.md"] = (
            f"# {title}\n\n{body}\n" if body else f"# {title}\n"
        )
        entries.append(
            f"- [{title}]({slug}.md)" + (f": {description}" if description else "")
        )
    if entries:
        pages["guides/index.md"] = "\n".join(["# Guides", "", *entries]) + "\n"
    return pages
