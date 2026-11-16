"""Find the relative links of a generated page the way a renderer reads them.

A link is ``[text](target.md#fragment)`` outside fenced code blocks and outside
inline code spans of any backtick length. An escaped bracket or backtick starts
nothing. The destination never holds whitespace, so a link cannot run across a
line. A code span may run over lines inside one paragraph, but never over a
table row, because a table row ends every span in it.

Page generators put untrusted text into code spans. Both consumers of links
(the link check and the Starlight rewrite) read links through this module, so
they agree with each other and with the renderer about what is code.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

_FENCE_OPEN = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
_BACKTICKS = re.compile(r"`+")
_DESTINATION = re.compile(r"\]\(([^)#:\s]+\.(?:md|json))(#[^)\s]*)?\)")


@dataclass(frozen=True)
class Link:
    """A link destination. ``start:end`` spans ``](target#fragment)``."""

    start: int
    end: int
    target: str
    fragment: str


def _scan(unit: str, base: int) -> list[Link]:
    links = []
    open_brackets = 0
    i = 0
    while i < len(unit):
        char = unit[i]
        if char == "\\":
            i += 2
        elif char == "`":
            run = _BACKTICKS.match(unit, i)
            assert run is not None
            closing = re.compile(rf"(?<!`)`{{{len(run[0])}}}(?!`)").search(
                unit, run.end()
            )
            i = closing.end() if closing else run.end()
        elif char == "]" and open_brackets:
            open_brackets -= 1
            match = _DESTINATION.match(unit, i)
            if match:
                links.append(
                    Link(
                        base + match.start(),
                        base + match.end(),
                        match[1],
                        match[2] or "",
                    )
                )
                i = match.end()
            else:
                i += 1
        else:
            open_brackets += char == "["
            i += 1
    return links


def find_links(text: str) -> list[Link]:
    """Every relative ``.md`` or ``.json`` link in ``text``, in order."""
    links: list[Link] = []
    fence: tuple[str, int] | None = None
    unit_start = 0
    unit = ""

    def flush() -> None:
        nonlocal unit, unit_start
        if unit:
            links.extend(_scan(unit, unit_start))
        unit = ""

    offset = 0
    for line in text.splitlines(keepends=True):
        bare = line.rstrip("\r\n")
        if fence:
            char, length = fence
            if re.fullmatch(rf" {{0,3}}{re.escape(char)}{{{length},}}[ \t]*", bare):
                fence = None
        elif (opening := _FENCE_OPEN.match(bare)) and not (
            opening[1][0] == "`" and "`" in opening[2]
        ):
            flush()
            fence = (opening[1][0], len(opening[1]))
        elif not bare.strip():
            flush()
        elif bare.lstrip().startswith("|"):
            flush()
            links.extend(_scan(line, offset))
        else:
            if not unit:
                unit_start = offset
            unit += line
        offset += len(line)
    flush()
    return links
