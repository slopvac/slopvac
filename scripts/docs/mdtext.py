"""Text helpers shared by the page generators: catalogue prose as safe Markdown."""

from __future__ import annotations

import html
import re


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


_BACKTICKS = re.compile(r"`+")
_MARKDOWN_SPECIALS = re.compile(r"([\\`*_\[\]()#!~])")


def code(value: object, *, in_table: bool = False) -> str:
    """A value as a literal Markdown code span, whatever characters it holds.

    The fence is one backtick longer than the longest backtick run inside, so
    the value cannot end the span early. Whitespace, including newlines, becomes
    single spaces. A value that starts or ends with a backtick or a space gets
    one space of padding. In a table cell, pipes are escaped.
    """
    body = " ".join(str(value).split())
    longest = max((len(run) for run in _BACKTICKS.findall(body)), default=0)
    fence = "`" * (longest + 1)
    if body[:1] in ("`", " ") or body[-1:] in ("`", " ") or not body:
        body = f" {body} "
    span = f"{fence}{body}{fence}"
    return span.replace("|", "\\|") if in_table else span


def untrusted(value: object) -> str:
    """Prose from outside the repository as one line of inert Markdown.

    HTML specials are escaped, and so is every character that starts Markdown
    syntax, so the text cannot become a link, an image or raw HTML.
    """
    return _MARKDOWN_SPECIALS.sub(r"\\\1", text(value))


def untrusted_cell(value: object) -> str:
    return untrusted(value).replace("|", "\\|")
