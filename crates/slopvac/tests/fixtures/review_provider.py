"""A model provider plugin for the `slopvac review` tests. Standard library only.

Usage: review_provider.py LOG_DIR

Each start appends a line to LOG_DIR/spawns, so a test can count spawns. The params of every
`provider/complete` request are appended to LOG_DIR/requests as one JSON line, and the body of
the request, the bytes as received, to LOG_DIR/requests.raw as one line. How the plugin
answers comes from LOG_DIR/script.json, which the test writes before each run:

- "order": "reverse" answers the items in reverse order.
- "text": {id: text} and "models": {id: model} set the answer of an item. The default text
  is "review of <id>" and the default model is the requested one.
- "omit", "duplicate", "no_text", "no_model", "empty_model", "bad_error": lists of item ids.
- "extra": ids to answer that were not requested.
- "errors": {id: code} answers the item with a JSON-RPC error object.
- "results": a value sent as `results` in place of the list.
- "request_error": a JSON-RPC code; the whole request fails with it.
- "offer_hooks": the hooks the `initialize` reply offers, in place of the one provider hook.
- "shutdown": "garbage" answers `shutdown` with a header line without a colon, which breaks
  the framing after every request has been answered.
"""

import json
import os
import sys

log_dir = sys.argv[1]
inp = sys.stdin.buffer
out = sys.stdout.buffer

with open(os.path.join(log_dir, "spawns"), "a") as f:
    f.write("spawn\n")


def script():
    try:
        with open(os.path.join(log_dir, "script.json")) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


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
    global last_body
    last_body = inp.read(length)
    return json.loads(last_body)


def send(obj):
    body = json.dumps(obj).encode()
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


def rpc_error(code):
    return {
        "code": code,
        "message": "PLUGIN-SECRET-MESSAGE",
        "data": {"slopvac_code": "review-provider-reply-invalid"},
    }


def answer(params, s):
    entries = []
    for item in params["items"]:
        i = item["id"]
        if i in s.get("omit", []):
            continue
        if i in s.get("errors", {}):
            entries.append({"item": i, "error": rpc_error(s["errors"][i])})
            continue
        if i in s.get("bad_error", []):
            entries.append({"item": i, "error": {"message": "no code"}})
            continue
        entry = {
            "item": i,
            "text": s.get("text", {}).get(i, "review of " + i),
            "model": s.get("models", {}).get(i, params["model"]),
        }
        if i in s.get("no_text", []):
            del entry["text"]
        if i in s.get("no_model", []):
            del entry["model"]
        if i in s.get("empty_model", []):
            entry["model"] = ""
        entries.append(entry)
        if i in s.get("duplicate", []):
            entries.append(dict(entry))
    for i in s.get("extra", []):
        entries.append({"item": i, "text": "extra", "model": params["model"]})
    if s.get("order") == "reverse":
        entries.reverse()
    return entries


INIT = {
    "protocolVersion": 1,
    "pluginInfo": {"name": "provider", "version": "1.0.0"},
    "capabilities": {
        "hooks": [{"kind": "provider", "id": "review"}],
        "features": {"required": [], "optional": []},
        "requiresNetwork": False,
    },
    "limits": {"maxMessageBytes": 1048576, "maxBatchDocuments": 64, "maxInFlight": 4},
}

while True:
    msg = read_msg()
    if msg is None:
        sys.exit(0)
    method = msg.get("method")
    id_ = msg.get("id")
    if method == "initialize":
        reply = json.loads(json.dumps(INIT))
        offered = script().get("offer_hooks")
        if offered is not None:
            reply["capabilities"]["hooks"] = offered
        send({"jsonrpc": "2.0", "id": id_, "result": reply})
    elif method == "provider/complete":
        params = msg["params"]
        with open(os.path.join(log_dir, "requests"), "a") as f:
            f.write(json.dumps(params) + "\n")
        with open(os.path.join(log_dir, "requests.raw"), "ab") as f:
            f.write(last_body + b"\n")
        s = script()
        if "request_error" in s:
            send({"jsonrpc": "2.0", "id": id_, "error": rpc_error(s["request_error"])})
        elif "results" in s:
            send({"jsonrpc": "2.0", "id": id_, "result": {"results": s["results"]}})
        else:
            send({"jsonrpc": "2.0", "id": id_, "result": {"results": answer(params, s)}})
    elif method == "shutdown":
        if script().get("shutdown") == "garbage":
            out.write(b"no colon in this header\r\n\r\n")
            out.flush()
        else:
            send({"jsonrpc": "2.0", "id": id_, "result": None})
    elif method == "exit":
        sys.exit(0)
    elif id_ is not None:
        send({"jsonrpc": "2.0", "id": id_, "error": rpc_error(-32601)})
