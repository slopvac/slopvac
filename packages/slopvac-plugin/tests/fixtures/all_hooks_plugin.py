"""Plugin that registers every hook kind, for the conformance tests."""

from slopvac_plugin import (
    Completion,
    Document,
    DocumentInvalid,
    Edit,
    Plugin,
    RecipeFile,
    Rendered,
    Segment,
    Tag,
)

plugin = Plugin(
    "all-hooks", "0.0.1", network=True, optional_features=("document-path",)
)


def reject_binary(doc: Document) -> None:
    if "\x00" in doc.text:
        raise DocumentInvalid("binary content")


@plugin.tagger("words")
def tag(doc, ctx):
    reject_binary(doc)
    return [Tag("prose", doc.span(0, len(doc.text)), 0.5, "whole document")]


@plugin.extractor("lines")
def extract(doc, ctx):
    reject_binary(doc)
    offset, segments = 0, []
    for line in doc.text.splitlines(keepends=True):
        segments.append(Segment(doc.span(offset, offset + len(line)), "prose"))
        offset += len(line)
    return segments


@plugin.reporter("plain")
def render(report, options, ctx):
    return Rendered(f"{len(report)} keys")


@plugin.fixer("upper")
def fix(doc, findings, ctx):
    reject_binary(doc)
    return [Edit(i, f.span, doc.slice(f.span).upper()) for i, f in enumerate(findings)]


@plugin.scorer("count")
def score(doc, findings, ctx):
    reject_binary(doc)
    return len(findings)


@plugin.provider("echo")
def complete(item, task, model, ctx):
    return Completion(item.text, model)


@plugin.recipe("files")
def plan(harness, options, ctx):
    return [RecipeFile(f"{harness}/AGENTS.md", "write", "# Agents\n")]


if __name__ == "__main__":
    plugin.run()
