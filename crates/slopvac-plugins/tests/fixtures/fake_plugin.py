"""Scripted fake plugin for the host conformance tests (H-tests).

Usage: fake_plugin.py MODE [OVERRIDES_JSON] [PIDFILE]

MODE picks a misbehaviour; OVERRIDES_JSON is deep-merged into the initialize
result. Standard library only.
"""

import json
import os
import subprocess
import sys
import time

mode = sys.argv[1] if len(sys.argv) > 1 else "ok"
overrides = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
pidfile = sys.argv[3] if len(sys.argv) > 3 else None
inp = sys.stdin.buffer
out = sys.stdout.buffer


def read_msg():
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
    body = inp.read(length)
    return json.loads(body)


def send(obj):
    body = json.dumps(obj).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


def result(id_, value):
    send({"jsonrpc": "2.0", "id": id_, "result": value})


def error(id_, code, slug, message="e"):
    send(
        {
            "jsonrpc": "2.0",
            "id": id_,
            "error": {"code": code, "message": message, "data": {"slopvac_code": slug}},
        }
    )


def merge(base, extra):
    for k, v in extra.items():
        if isinstance(v, dict) and isinstance(base.get(k), dict):
            merge(base[k], v)
        else:
            base[k] = v
    return base


def init_result():
    base = {
        "protocolVersion": 1,
        "pluginInfo": {"name": "fake", "version": "0.1"},
        "capabilities": {
            "hooks": [
                {"kind": "tagger", "id": "main"},
                {"kind": "fixer", "id": "main"},
            ],
            "features": {"required": [], "optional": []},
            "requiresNetwork": False,
        },
        "limits": {
            "maxMessageBytes": 1048576,
            "maxBatchDocuments": 64,
            "maxInFlight": 4,
        },
    }
    return merge(base, overrides)


def tags_for(params):
    results = []
    for doc in params["documents"]:
        text = doc["text"]
        evidence = "ok"
        if mode == "env":
            evidence = json.dumps(dict(os.environ))
        if mode == "paths":
            evidence = json.dumps(doc.get("path"))
        results.append(
            {
                "document": doc["id"],
                "tags": [
                    {
                        "kind": "code-fence",
                        "confidence": 1,
                        "span": {"start": 0, "end": len(text.encode())},
                        "evidence": evidence,
                    }
                ],
            }
        )
    return {"results": results}


def spawn_children():
    pids = [subprocess.Popen(["sleep", "300"]).pid]
    pids.append(subprocess.Popen(["sleep", "300"], start_new_session=True).pid)
    if pidfile:
        with open(pidfile, "w") as f:
            f.write("\n".join(map(str, pids)))


if mode == "stray-stdout":
    out.write(b"hello\r\n")
    out.flush()

shut = False
pending_tag = None
while True:
    msg = read_msg()
    if msg is None:
        sys.exit(0)
    method = msg.get("method")
    id_ = msg.get("id")
    if method == "initialize":
        if mode == "silent-init":
            time.sleep(300)
        if mode == "version-unsupported":
            error(id_, -32001, "protocol-version-unsupported")
            continue
        result(id_, init_result())
    elif method == "initialized":
        if mode == "flood":
            for _ in range(1500):
                send({"jsonrpc": "2.0", "method": "noise", "params": {}})
        if mode == "children" or mode == "ignore-shutdown":
            spawn_children()
        if mode == "no-read":
            time.sleep(300)
        if mode == "stderr-flood":
            sys.stderr.write("x" * (10 * 1024 * 1024))
            sys.stderr.flush()
    elif method == "shutdown":
        if mode == "ignore-shutdown":
            time.sleep(300)
        shut = True
        result(id_, None)
    elif method == "exit":
        sys.exit(0 if shut else 1)
    elif method == "$/cancelRequest":
        if mode == "cancel-ok" and pending_tag is not None:
            error(pending_tag, -32005, "request-cancelled")
            pending_tag = None
        elif mode == "cancel-late-answer" and pending_tag is not None:
            result(pending_tag, tags_for(pending_params))
            pending_tag = None
    elif method == "tagger/tag":
        if mode in ("hang", "cancel-ok", "cancel-late-answer"):
            pending_tag = id_
            pending_params = msg["params"]
        elif mode == "exit-mid":
            sys.stderr.write("boom \x1b[31mred\x1b[0m\n")
            sys.stderr.flush()
            sys.exit(3)
        elif mode == "partial-frame":
            out.write(b"Content-Length: 100\r\n\r\n{")
            out.flush()
            time.sleep(300)
        elif mode == "oversize":
            out.write(b"Content-Length: 99999999\r\n\r\n")
            out.flush()
            time.sleep(300)
        elif mode == "doc-error":
            docs = msg["params"]["documents"]
            res = tags_for(msg["params"])
            res["results"][1] = {
                "document": docs[1]["id"],
                "error": {
                    "code": -32006,
                    "message": "binary",
                    "data": {"slopvac_code": "document-invalid"},
                },
            }
            result(id_, res)
        elif mode == "bad-span":
            res = tags_for(msg["params"])
            res["results"][0]["tags"][0]["span"]["end"] = 10_000
            result(id_, res)
        elif mode == "missing-doc":
            res = tags_for(msg["params"])
            res["results"] = res["results"][:-1]
            result(id_, res)
        elif mode == "unknown-id":
            result(9999, {"results": []})
            result(id_, tags_for(msg["params"]))
        else:
            result(id_, tags_for(msg["params"]))
    elif method == "fixer/fix":
        findings_by_doc = msg["params"]["documents"]
        results = []
        for doc in findings_by_doc:
            f = doc["findings"][0]["span"]
            start = f["start"] - 1 if mode == "fix-outside" else f["start"]
            results.append(
                {
                    "document": doc["id"],
                    "edits": [
                        {
                            "finding": 0,
                            "span": {"start": start, "end": f["end"]},
                            "text": "fixed",
                        }
                    ],
                }
            )
        result(id_, {"results": results})
    elif id_ is not None:
        error(id_, -32601, "method-not-found")
