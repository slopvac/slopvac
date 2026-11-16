"""Adapt the neutral generated pages for the Starlight site under ``site/``.

Starlight needs a ``title`` in front matter and renders the page title itself,
so the leading ``# Title`` line moves into front matter. Relative links between
pages are rewritten from file paths to route paths. Routes end in a slash, so a
link from ``rules/index.md`` to ``rules/x/index.md`` becomes ``x/``. Raw schema
files go to ``public/schemas/<api>/`` and link by relative path, so links work
under any base path.
"""

from __future__ import annotations

import json
import posixpath
import re
import shutil
from pathlib import Path

import mdlinks

Pages = dict[str, str]

_TITLE = re.compile(r"\A# (.+)\n+")


def _route(path: str, api_version: str) -> str:
    """Site-relative route (no leading slash) for a neutral page path."""
    if path.endswith(".json"):
        return path
    stem = path.removesuffix(".md")
    if posixpath.basename(stem) == "index":
        stem = posixpath.dirname(stem)
    return posixpath.join(api_version, stem).rstrip("/") + "/"


def _rewrite_links(path: str, text: str, api_version: str) -> str:
    # Every page URL ends in a slash, so its links resolve against the page route itself.
    start = _route(path, api_version).rstrip("/")

    def replace(link: mdlinks.Link) -> str:
        target = posixpath.normpath(
            posixpath.join(posixpath.dirname(path), link.target)
        )
        route = _route(target, api_version)
        relative = posixpath.relpath(route.rstrip("/"), start or ".")
        if route.endswith("/"):
            relative += "/"
        return f"]({relative}{link.fragment})"

    # Links in code, and text that is not a link, stay as written.
    for link in reversed(mdlinks.find_links(text)):
        text = text[: link.start] + replace(link) + text[link.end :]
    return text


def _with_front_matter(text: str) -> str:
    match = _TITLE.match(text)
    if not match:
        raise ValueError("generated page has no leading '# Title' line")
    return f"---\ntitle: {json.dumps(match[1])}\n---\n\n{text[match.end() :]}"


def write_site(
    pages: Pages, site_dir: Path, api_version: str, site_url: str, base: str
) -> None:
    """Write pages, raw schemas and the site config into ``site_dir``."""
    content = site_dir / "src" / "content" / "docs" / api_version
    public = site_dir / "public" / "schemas"
    for directory in (content, public):
        if directory.exists():
            shutil.rmtree(directory)
        directory.mkdir(parents=True)
    for path, text in pages.items():
        if path.endswith(".json"):
            target = site_dir / "public" / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)
            continue
        target = content / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(_with_front_matter(_rewrite_links(path, text, api_version)))
    generated = site_dir / "generated"
    generated.mkdir(exist_ok=True)
    (generated / "site.json").write_text(
        json.dumps({"site": site_url, "base": base}) + "\n"
    )
