"""Text helpers shared by the page generators: catalogue prose as safe Markdown."""

from __future__ import annotations

import html


def one_line(text: object) -> str:
    return " ".join(str(text).split())


def text(value: object) -> str:
    """Catalogue prose as one Markdown line, with HTML specials escaped.

    Unicode dashes become the ASCII ``--`` that the prose rules ask for.
    """
    collapsed = one_line(value).replace("\u2014", "--").replace("\u2013", "--")
    return html.escape(collapsed, quote=False)


def cell(value: object) -> str:
    return text(value).replace("|", "\\|")
