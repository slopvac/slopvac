"""Fixer that inserts twice at the start of each finding (positions.md: array order)."""

from slopvac_plugin import Edit, Plugin, Span

plugin = Plugin("double-insert", "0.0.1")


@plugin.fixer("twice")
def fix(doc, findings, ctx):
    edits = []
    for i, finding in enumerate(findings):
        at = Span(finding.span.start, finding.span.start)
        edits += [Edit(i, at, "X"), Edit(i, at, "Y")]
    return edits


if __name__ == "__main__":
    plugin.run()
