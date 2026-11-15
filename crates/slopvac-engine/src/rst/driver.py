"""Read reStructuredText on stdin and write its prose nodes as JSON on stdout.

slopvac runs this file as `python -E -B -X utf8 -c <this text>` and maps the output back
to source positions. The document is untrusted: file insertion and raw output are off,
no configuration file is read, and system messages stay out of the result.

The parser runs without transforms, so the tree is the source as written: no generated
table of contents, no substitutions, no admonition titles. Every node that holds prose
carries the source lines docutils recorded for it, and each inline child keeps its raw
source text (`rawsource`) so that the Rust side can place the words.

Exit codes: 3 docutils is missing, 4 it is older than 0.21, 5 the document nests too deep.
"""

import io
import json
import re
import sys

OLDEST = (0, 21)


def main():
    try:
        import docutils
        from docutils import nodes
        from docutils.frontend import get_default_settings
        from docutils.parsers.rst import Parser
        from docutils.utils import new_document
    except ImportError:
        return 3
    if tuple(docutils.__version_info__[:2]) < OLDEST:
        return 4

    source = sys.stdin.buffer.read().decode("utf-8")
    # docutils splits lines with str.splitlines, which also breaks at these characters.
    # slopvac counts LF, CRLF and CR only, so make the line numbers agree.
    source = re.sub("[\x1c-\x1e\x85\u2028\u2029]", " ", source)

    settings = get_default_settings(Parser)
    settings.file_insertion_enabled = False
    settings.raw_enabled = False
    settings._disable_config = True
    settings.report_level = 5
    settings.halt_level = 5
    settings.warning_stream = io.StringIO()
    settings.syntax_highlight = "none"
    document = new_document("<stdin>", settings)
    try:
        Parser().parse(source, document)
    except RecursionError:
        return 5

    skip_subtree = {"system_message", "comment", "raw"}
    skip_own = {
        "field_name",
        "label",
        "option_string",
        "option_argument",
        "problematic",
    }
    code_tags = {"literal_block", "doctest_block", "math_block"}
    opaque = {
        "footnote_reference",
        "citation_reference",
        "substitution_reference",
        "problematic",
        "image",
    }

    def lines_of(node):
        start = node.line
        if start is None:
            return None, None
        if node.tagname == "title":
            return start - 1, start - 1
        raw = (node.rawsource or "").rstrip("\n")
        return start, start + raw.count("\n")

    def containers(node):
        found = []
        depth = 0
        chain = []
        parent = node.parent
        while parent is not None:
            chain.append(parent)
            parent = parent.parent
        for ancestor in reversed(chain):
            if isinstance(ancestor, (nodes.bullet_list, nodes.enumerated_list)):
                depth += 1
            elif isinstance(ancestor, nodes.list_item):
                ordered = isinstance(ancestor.parent, nodes.enumerated_list)
                found.append({"t": "li", "ordered": ordered, "depth": depth})
            elif isinstance(ancestor, nodes.block_quote):
                found.append({"t": "bq"})
        return found

    def cell_of(node):
        entry = node.parent
        if not isinstance(entry, nodes.entry):
            return None
        row = entry.parent
        group = row.parent
        before = 0
        for sibling in group.parent.children:
            if sibling is group:
                break
            if isinstance(sibling, (nodes.thead, nodes.tbody)):
                before += len(sibling.children)
        return [before + group.index(row), row.index(entry)]

    def section_level(node):
        parent = node.parent
        if not isinstance(parent, nodes.section):
            return None
        level = 0
        while parent is not None:
            if isinstance(parent, nodes.section):
                level += 1
            parent = parent.parent
        return min(level, 6)

    def pieces_of(node):
        pieces = []
        for child in node.children:
            if isinstance(child, nodes.Text):
                pieces.append({"k": "t", "t": child.astext()})
                continue
            if not isinstance(child, nodes.Inline):
                continue
            raw = child.rawsource or ""
            tag = child.tagname
            if tag == "target":
                continue  # Its source lies inside the previous reference.
            if tag in ("literal", "math"):
                pieces.append({"k": "c", "t": child.astext(), "raw": raw})
            elif tag in opaque:
                if raw:
                    pieces.append({"k": "o", "raw": raw})
            else:
                pieces.append({"k": "i", "t": child.astext(), "raw": raw})
        return pieces

    def has_prose(node):
        for child in node.children:
            if isinstance(child, nodes.Text) and child.astext().strip():
                return True
            if isinstance(child, nodes.Inline) and child.tagname not in ("target",):
                return True
        return False

    out = []
    # Post-order bookkeeping for the admonition extents.
    admonitions = []
    stack = [document]
    while stack:
        node = stack.pop()
        tag = node.tagname
        if tag in skip_subtree:
            continue
        if isinstance(node, nodes.Admonition):
            start = node.line
            entry = {
                "tag": "admonition",
                "adm": tag,
                "start": start,
                "end": start,
                "title": next(
                    (c.astext() for c in node.children if isinstance(c, nodes.title)),
                    None,
                ),
                "containers": containers(node),
            }
            out.append(entry)
            admonitions.append((entry, node))
        elif tag in code_tags:
            start, end = lines_of(node)
            text = node.astext()
            if start is not None:
                end = start + text.rstrip("\n").count("\n")
                classes = node.get("classes", [])
                lang = classes[1] if len(classes) > 1 and classes[0] == "code" else None
                out.append(
                    {
                        "tag": tag,
                        "code": True,
                        "start": start,
                        "end": end,
                        "text": text,
                        "lang": lang,
                        "containers": containers(node),
                    }
                )
            continue
        elif (
            tag not in skip_own
            and isinstance(node, nodes.TextElement)
            and has_prose(node)
        ):
            parent = node.parent
            if tag == "title" and isinstance(parent, nodes.Admonition):
                pass  # Masked with the admonition, like the other parsers do.
            else:
                start, end = lines_of(node)
                entry = {
                    "tag": tag,
                    "start": start,
                    "end": end,
                    "pieces": pieces_of(node),
                    "containers": containers(node),
                }
                if tag == "title":
                    entry["level"] = section_level(node)
                cell = cell_of(node) if tag == "paragraph" else None
                if cell is not None:
                    entry["cell"] = cell
                out.append(entry)
        for child in reversed(node.children):
            if isinstance(child, nodes.Element) and not isinstance(child, nodes.Inline):
                stack.append(child)

    # An admonition runs to the last line of anything inside it.
    for entry, node in admonitions:
        last = entry["start"]
        inner = [node]
        while inner:
            current = inner.pop()
            if current.line is not None:
                _, end = lines_of(current)
                if end is not None:
                    last = max(last, end)
            inner.extend(
                c
                for c in current.children
                if isinstance(c, nodes.Element) and not isinstance(c, nodes.Inline)
            )
        entry["end"] = last

    # Admonitions come before their children in source order.
    out.sort(key=lambda e: e["start"] if e["start"] is not None else 10**9)

    result = {"docutils": docutils.__version__, "nodes": out}
    sys.stdout.buffer.write(json.dumps(result, ensure_ascii=False).encode("utf-8"))
    return 0


sys.exit(main())
