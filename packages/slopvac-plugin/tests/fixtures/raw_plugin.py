"""Protocol v1 plugin without the SDK, with optional misbehaviour.

Usage: raw_plugin.py [MODE]. Modes: no-limit, stdout-noise, bad-span, split-char-span,
exit-zero-without-shutdown, no-version-check, read-oversized-body, slow-tagger,
float-id, float-limit, big-init-reply, big-reply, deaf-after-init, in-flight-17, in-flight-unsafe, unknown-kind,
mixed-features, many-hooks, call-oversized, small-limit (maxMessageBytes 65536),
batch1 (maxBatchDocuments 1) and its variants batch1-swapped, batch1-both,
batch1-malformed and batch1-slow. Standard library only, like a plugin in
any other language would be.
"""

import json
import sys
import time

mode = sys.argv[1] if len(sys.argv) > 1 else "ok"
inp, out = sys.stdin.buffer, sys.stdout.buffer
limit = 1048576
state = "new"


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
        if mode == "small-limit":
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
                    "hooks": [{"kind": "tagger", "id": "main"}]
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
                    if mode == "small-limit"
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
        if params.get("tagger") != "main":
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
