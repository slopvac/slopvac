"""Deterministic text operations shared by the generators.

Standard library only. Nothing here calls a model.
"""

from __future__ import annotations

import re

FENCE_RE = re.compile(r"^\s*(```|~~~)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
CODE_SPAN_RE = re.compile(r"`([^`\n]{2,40})`")
NUMBER_RE = re.compile(r"(?<![\w.`/-])(\d{1,4})(?![\w.`/%-])")
SENTENCE_BREAK_RE = re.compile(r"(?<=[.!?])\s+(?=[A-Z`\"(])")
NON_PROSE_START = ("#", "-", "*", "+", ">", "|", "<", "{", "[", "!", ":", "=", "@", "$")


def prose_paragraphs(text: str, min_words: int = 12) -> list[tuple[int, int]]:
    """Return (first, last) 0-based line indexes of plain prose paragraphs.

    A paragraph is a run of non-blank lines outside code fences that does not
    start like a heading, list, quote, table, HTML or front matter, and that
    ends a sentence. Markdown and reStructuredText prose both qualify.
    """
    lines = text.split("\n")
    found: list[tuple[int, int]] = []
    in_fence = False
    in_front = bool(lines) and lines[0].strip() == "---"
    start: int | None = None

    def close(end: int) -> None:
        nonlocal start
        if start is not None:
            chunk = lines[start : end + 1]
            body = " ".join(chunk)
            first = chunk[0]
            if (
                len(body.split()) >= min_words
                and not first.startswith((" ", "\t") + NON_PROSE_START)
                and not re.match(r"\d+[.)]\s", first)
                and body.rstrip().endswith((".", "!", "?"))
                and "{{" not in body
                and "{%" not in body
            ):
                found.append((start, end))
        start = None

    for i, line in enumerate(lines):
        if in_front:
            if i > 0 and line.strip() in ("---", "..."):
                in_front = False
            continue
        if FENCE_RE.match(line):
            close(i - 1)
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if not line.strip():
            close(i - 1)
        elif start is None:
            start = i
    close(len(lines) - 1)
    return found


def split_sentences(paragraph: str) -> list[str]:
    return SENTENCE_BREAK_RE.split(paragraph.strip())


def outline_for(text: str, genre: str, fmt: str) -> str:
    """Describe a Markdown document by structure and facts, never by wording.

    The outline lists the sections, a word budget per section, and the code
    spans and numbers a section mentions. A model that writes from it never
    sees a human sentence.
    """
    lines = text.split("\n")
    sections: list[tuple[str, list[str]]] = [("Introduction", [])]
    in_fence = False
    for line in lines:
        if FENCE_RE.match(line):
            in_fence = not in_fence
        match = None if in_fence else HEADING_RE.match(line)
        if match:
            sections.append((match.group(2), []))
        else:
            sections[-1][1].append(line)
    if not sections[0][1] or not "".join(sections[0][1]).strip():
        sections = sections[1:] or sections
    total = len(text.split())
    out = [
        f"Document type: {genre}",
        f"Format: {fmt}",
        f"Target length: about {max(50, round(total, -1))} words",
        "",
        "Sections, in order:",
    ]
    for number, (heading, body_lines) in enumerate(sections, 1):
        body = "\n".join(body_lines)
        words = len(body.split())
        facts: list[str] = []
        for token in CODE_SPAN_RE.findall(body) + NUMBER_RE.findall(body):
            if token not in facts:
                facts.append(token)
        out.append(f"{number}. {heading} (about {max(10, round(words, -1))} words)")
        if facts:
            shown = ", ".join(f"`{f}`" for f in facts[:8])
            out.append(f"   Mention: {shown}")
    return "\n".join(out) + "\n"
