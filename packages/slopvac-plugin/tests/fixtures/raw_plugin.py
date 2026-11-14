"""Protocol v1 plugin without the SDK, with optional misbehaviour.

Usage: raw_plugin.py [MODE]. Modes: no-limit, stdout-noise, bad-span, split-char-span,
exit-zero-without-shutdown, no-version-check, read-oversized-body, slow-tagger,
float-id, float-limit, big-init-reply, big-reply, deaf-after-init, in-flight-17, in-flight-unsafe, unknown-kind,
mixed-features, many-hooks, call-oversized, small-limit (maxMessageBytes 65536),
batch1 (maxBatchDocuments 1) and its variants batch1-swapped, batch1-both,
batch1-malformed and batch1-slow, ignore-eof (a slow initialize reply, and end of
input does not end it), nine-taggers (taggers t1 to t9 under the small limit, and an
error entry for the document "bad"), escape-silent, trickle-after-exit and
drip-after-exit. The last three start a descendant in a new session that keeps
standard output open: one stays silent, one prints a frame every 0.4 s once the
plugin exits, and one then sends a header and a body one byte at a time. They append its
pid to the file named by ESCAPE_PIDFILE. Standard library only, like a plugin in
any other language would be.
"""

import atexit
import json
import os
import subprocess
import sys
import time

mode = sys.argv[1] if len(sys.argv) > 1 else "ok"
inp, out = sys.stdin.buffer, sys.stdout.buffer
limit = 1048576
state = "new"
TAGGERS = [f"t{n}" for n in range(1, 10)] if mode == "nine-taggers" else ["main"]
TRICKLE = """
import sys, time
body = b'{"jsonrpc":"2.0","method":"x-late","params":{}}'
for _ in range(75):
    sys.stdout.buffer.write(b"Content-Length: %d\\r\\n\\r\\n" % len(body) + body)
    sys.stdout.buffer.flush()
    time.sleep(0.4)
"""
# A body that never completes: one byte every 0.2 s.
DRIP = """
import sys, time
sys.stdout.buffer.write(b"Content-Length: 1000000\\r\\n\\r\\n")
for _ in range(150):
    sys.stdout.buffer.write(b"x")
    sys.stdout.buffer.flush()
    time.sleep(0.2)
"""


def escape(code):
    """Start a descendant that leaves the process group and holds standard output."""
    child = subprocess.Popen(
        [sys.executable, "-c", code],
        stdin=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    with open(os.environ["ESCAPE_PIDFILE"], "a", encoding="utf-8") as handle:
        handle.write(f"{child.pid}\n")


if mode == "escape-silent":
    escape("import time; time.sleep(30)")
if mode == "trickle-after-exit":
    atexit.register(escape, TRICKLE)
if mode == "drip-after-exit":
    atexit.register(escape, DRIP)


def scalar_after_exit():
    # A framed JSON string with terminal controls, a bidi override and a zero-width space.
    body = json.dumps("\x1b[31mred\u202eevil\u200bz\x07").encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


if mode == "scalar-after-exit":
    atexit.register(scalar_after_exit)


def read():
    length = None
    while True:
        line = inp.readline()
        if not line:
            return None
        line = line.rstrip(b"\r\n")
        if not line:
            break
        name, _, value = line.partition(b":")
        if name.lower() == b"content-length":
            length = int(value)
    if mode == "read-oversized-body" and state != "new":
        body = inp.read(length)  # after initialize it reads first and rejects after
        if length > limit:
            sys.exit(1)
        return json.loads(body)
    if length > limit and mode != "no-limit":
        sys.exit(1)
    return json.loads(inp.read(length))


def send(obj):
    body = json.dumps(obj).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


def reply(i, result):
    send(
        {
            "jsonrpc": "2.0",
            "id": float(i) if mode == "float-id" else i,
            "result": result,
        }
    )


def fail(i, code, slug, **data):
    send(
        {
            "jsonrpc": "2.0",
            "id": i,
            "error": {
                "code": code,
                "message": "e",
                "data": {"slopvac_code": slug, **data},
            },
        }
    )


while True:
    msg = read()
    if msg is None and mode == "ignore-eof":
        time.sleep(60)
    if msg is None:
        sys.exit(0 if state == "shutdown" else 1)
    method, i, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
    if method == "exit":
        sys.exit(
            0 if state == "shutdown" or mode == "exit-zero-without-shutdown" else 1
        )
    if i is None:
        continue
    if method == "initialize":
        if mode != "no-version-check" and params["minProtocolVersion"] > 1:
            fail(
                i,
                -32001,
                "protocol-version-unsupported",
                supported={"min": 1, "max": 1},
            )
            continue
        limit = params["limits"]["maxMessageBytes"]
        if mode == "ignore-eof":
            time.sleep(1)
        small = mode in ("small-limit", "nine-taggers")
        if small:
            limit = 65536  # a frame over it ends this plugin
        reply(
            i,
            {
                "protocolVersion": 1,
                "pluginInfo": {
                    "name": "raw" + "a" * (1100000 if mode == "big-init-reply" else 0),
                    "version": "0",
                },
                "capabilities": {
                    "hooks": [{"kind": "tagger", "id": t} for t in TAGGERS]
                    + (
                        [{"kind": "hologram", "id": "x"}]
                        if mode == "unknown-kind"
                        else []
                    )
                    + (
                        [{"kind": "tagger", "id": f"h{n}"} for n in range(25000)]
                        if mode == "many-hooks"
                        else []
                    ),
                    "features": {
                        "required": [],
                        # Distinct strings and one integer: the schema's uniqueItems
                        # check compares such an array pair by pair.
                        "optional": [f"f{n}" for n in range(30000)] + [1]
                        if mode == "mixed-features"
                        else [],
                    },
                    "requiresNetwork": False,
                },
                "limits": {
                    "maxMessageBytes": float(limit)
                    if mode == "float-limit"
                    else 65536
                    if small
                    else limit,
                    "maxBatchDocuments": 1 if mode.startswith("batch1") else 64,
                    "maxInFlight": {"in-flight-17": 17, "in-flight-unsafe": 2**60}.get(
                        mode, 1
                    ),
                },
            },
        )
        state = "ready"
        if mode == "deaf-after-init":
            time.sleep(60)
    elif state == "new":
        fail(i, -32003, "not-initialized")
    elif state == "shutdown":
        fail(i, -32600, "invalid-request")
    elif method == "shutdown":
        reply(i, None)
        state = "shutdown"
    elif method == "tagger/tag":
        if params.get("tagger") not in TAGGERS:
            fail(i, -32602, "invalid-params")
            continue
        if mode == "call-oversized":
            # The v1 protocol has no plugin-to-host calls (section 3).
            send(
                {
                    "jsonrpc": "2.0",
                    "id": 99,
                    "method": "tagger/tag",
                    "params": {
                        "documents": [{"id": f"d{n}", "text": ""} for n in range(20000)]
                        + [1]
                    },
                }
            )
            continue
        if mode == "slow-tagger":
            time.sleep(3)  # ignores the cancel
        if mode == "stdout-noise":
            out.write(b"debug: handling a request\n")
            out.flush()
        if mode == "batch1-slow":
            time.sleep(0.3)
        ids = [doc["id"] for doc in params["documents"]]
        if mode == "batch1-malformed" and ids == ["doc-2"]:
            reply(i, {"nope": 1})  # the second batch gets an answer with no results
            continue
        if mode == "batch1-both" and ids == ["doc-2"]:
            reply(i, {"results": []})  # doc-2 was answered in the first reply
            continue
        results = []
        documents = list(params["documents"])
        if mode == "batch1-both":
            documents = [{"id": "doc-1", "text": ""}, {"id": "doc-2", "text": ""}]
        for doc in documents:
            if mode == "nine-taggers" and doc["id"] == "bad":
                error = {
                    "code": -32602,
                    "message": "bad",
                    "data": {"slopvac_code": "x"},
                }
                results.append({"document": "bad", "error": error})
                continue
            size = len(doc["text"].encode())
            end = {"bad-span": size + 1, "split-char-span": 2}.get(mode, size)
            swapped = {"doc-1": "doc-2", "doc-2": "doc-1"}
            results.append(
                {
                    "document": swapped[doc["id"]]
                    if mode == "batch1-swapped"
                    else doc["id"],
                    "tags": [
                        {
                            "kind": "prose",
                            "span": {"start": 0, "end": end},
                            "confidence": 1,
                            "evidence": "e" * (70000 if mode == "big-reply" else 1),
                        }
                    ],
                }
            )
        reply(i, {"results": results})
    else:
        fail(i, -32601, "method-not-found")
