"""Example tagger: marks Markdown fenced code blocks with the ``code`` context tag."""

from slopvac_plugin import Context, Document, DocumentInvalid, Plugin, Tag

plugin = Plugin("fence-tagger", "0.1.0")


@plugin.tagger("fences")
def tag_fences(doc: Document, ctx: Context) -> list[Tag]:
    if "\x00" in doc.text:
        raise DocumentInvalid("binary content")
    tags: list[Tag] = []
    start: int | None = None
    offset = 0
    for line in doc.text.splitlines(keepends=True):
        if line.lstrip().startswith("```"):
            if start is None:
                start = offset
            else:
                tags.append(_tag(doc, start, offset + len(line)))
                start = None
        offset += len(line)
    if start is not None:  # an unclosed fence runs to the end of the document
        tags.append(_tag(doc, start, len(doc.text)))
    print(f"tagged {len(tags)} fenced blocks")  # stdout is redirected to stderr
    return tags


def _tag(doc: Document, start: int, end: int) -> Tag:
    return Tag("code", doc.span(start, end), 1.0, "fenced code block")


if __name__ == "__main__":
    plugin.run()
