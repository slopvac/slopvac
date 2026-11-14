"""Plugin with hooks that misbehave in one way each, for SDK behaviour tests.

Usage: behaviour_plugin.py [needs-feature | in-flight-2]
"""

import logging
import os
import subprocess
import sys
import threading
import time

from slopvac_plugin import Plugin, Span, Tag

mode = sys.argv[1] if len(sys.argv) > 1 else ""
plugin = Plugin(
    "behaviour",
    "0.0.1",
    required_features=("x-needed",) if mode == "needs-feature" else (),
    optional_features=("document-path",),
    max_in_flight=2 if mode == "in-flight-2" else 1,
)
gate = threading.Barrier(2, timeout=5)


@plugin.tagger("slow")
def slow(doc, ctx):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        ctx.raise_if_cancelled()
        time.sleep(0.01)
    return []


@plugin.tagger("stuck")
def stuck(doc, ctx):
    time.sleep(30)  # ignores cancellation
    return []


@plugin.tagger("gate")
def meet(doc, ctx):
    gate.wait()  # both requests must be in flight at once
    return []


@plugin.tagger("bad-span")
def bad_span(doc, ctx):
    return [Tag("prose", Span(0, len(doc.data) + 1), 1.0, "past the end")]


@plugin.tagger("huge")
def huge(doc, ctx):
    return [Tag("prose", doc.span(0, 0), 1.0, "x" * 70000)]


@plugin.tagger("crash")
def crash(doc, ctx):
    raise RuntimeError("secret document text must not reach the log")


@plugin.tagger("chatty")
def chatty(doc, ctx):
    print("print to stdout")
    logging.getLogger().warning("log line")
    os.write(1, b"raw write to descriptor 1\n")
    subprocess.run([sys.executable, "-c", "print('child output')"], check=True)
    return []


@plugin.tagger("features")
def features(doc, ctx):
    return [Tag("prose", doc.span(0, 0), 1.0, ",".join(sorted(ctx.features)))]


if __name__ == "__main__":
    plugin.run()
