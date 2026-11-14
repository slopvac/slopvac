"""Protocol v1 plugin without the SDK, with optional misbehaviour.

Usage: raw_plugin.py [MODE]. Modes: no-limit, stdout-noise, bad-span, split-char-span,
exit-zero-without-shutdown, no-version-check. Standard library only, like a plugin in
any other language would be.
"""

import json
import sys

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
    if length > limit and mode != "no-limit":
        sys.exit(1)
    return json.loads(inp.read(length))


def send(obj):
    body = json.dumps(obj).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


def reply(i, result):
    send({"jsonrpc": "2.0", "id": i, "result": result})


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
        reply(
            i,
            {
                "protocolVersion": 1,
                "pluginInfo": {"name": "raw", "version": "0"},
                "capabilities": {
                    "hooks": [{"kind": "tagger", "id": "main"}],
                    "features": {"required": [], "optional": []},
                    "requiresNetwork": False,
                },
                "limits": {
                    "maxMessageBytes": limit,
                    "maxBatchDocuments": 64,
                    "maxInFlight": 1,
                },
            },
        )
        state = "ready"
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
        if mode == "stdout-noise":
            out.write(b"debug: handling a request\n")
            out.flush()
        results = []
        for doc in params["documents"]:
            size = len(doc["text"].encode())
            end = {"bad-span": size + 1, "split-char-span": 2}.get(mode, size)
            results.append(
                {
                    "document": doc["id"],
                    "tags": [
                        {
                            "kind": "prose",
                            "span": {"start": 0, "end": end},
                            "confidence": 1,
                            "evidence": "e",
                        }
                    ],
                }
            )
        reply(i, {"results": results})
    else:
        fail(i, -32601, "method-not-found")
