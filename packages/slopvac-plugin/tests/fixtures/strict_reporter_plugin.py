"""A reporter that reads the fields a lint report always carries."""

from slopvac_plugin import Plugin, Rendered

plugin = Plugin("strict-reporter", "0.0.1")


@plugin.reporter("strict")
def render(report, options, ctx):
    summary = report["summary"]
    lines = [f"{summary['documents']} documents, {summary['findings']} findings"]
    for document in report["documents"]:
        lines.append(f"{document['path']}: {document['total_findings']}")
    return Rendered("\n".join(lines))


if __name__ == "__main__":
    plugin.run()
