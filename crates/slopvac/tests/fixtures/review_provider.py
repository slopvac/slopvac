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
  the framing after every request has been answered. "error" answers it with a JSON-RPC error
  object, "exit" leaves without an answer, and "not_null" answers it with a value that is not
  null.
- "init": how `initialize` fails. {"error": code} answers with a JSON-RPC error object, and
  {"error": code, "supported": text} adds `data.supported`. "exit" writes PLUGIN-SECRET-MESSAGE
  to standard error and leaves. "duplicate_key" repeats the key PLUGIN-SECRET-KEY in the reply.
  "required_feature" lists PLUGIN-SECRET-FEATURE as a required feature.
- "complete": how `provider/complete` fails. "exit" writes PLUGIN-SECRET-MESSAGE to standard
  error and leaves; "duplicate_key" repeats the key PLUGIN-SECRET-KEY in an entry.
- "stderr": a line written to standard error before the answer to `provider/complete`, which
  the plugin then sends as usual.
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
    send_raw(json.dumps(obj).encode())


def send_raw(body):
    out.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    out.flush()


def leave_with_secret():
    sys.stderr.write("PLUGIN-SECRET-MESSAGE\n")
    sys.stderr.flush()
    sys.exit(3)


def with_duplicate_key(id_, result):
    """A response whose result object repeats the key PLUGIN-SECRET-KEY."""
    inner = json.dumps(result)
    assert inner.endswith("}")
    dup = inner[:-1] + ', "PLUGIN-SECRET-KEY": 1, "PLUGIN-SECRET-KEY": 2}'
    return ('{"jsonrpc": "2.0", "id": %s, "result": %s}' % (json.dumps(id_), dup)).encode()


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
        init = script().get("init")
        if isinstance(init, dict) and "error" in init:
            error = rpc_error(init["error"])
            if "supported" in init:
                error["data"]["supported"] = init["supported"]
            send({"jsonrpc": "2.0", "id": id_, "error": error})
        elif init == "exit":
            leave_with_secret()
        elif init == "duplicate_key":
            send_raw(with_duplicate_key(id_, reply))
        else:
            if init == "required_feature":
                reply["capabilities"]["features"]["required"] = ["PLUGIN-SECRET-FEATURE"]
            send({"jsonrpc": "2.0", "id": id_, "result": reply})
    elif method == "provider/complete":
        params = msg["params"]
        with open(os.path.join(log_dir, "requests"), "a") as f:
            f.write(json.dumps(params) + "\n")
        with open(os.path.join(log_dir, "requests.raw"), "ab") as f:
            f.write(last_body + b"\n")
        s = script()
        if "stderr" in s:
            sys.stderr.write(s["stderr"] + "\n")
            sys.stderr.flush()
        if s.get("complete") == "exit":
            leave_with_secret()
        elif s.get("complete") == "duplicate_key":
            send_raw(with_duplicate_key(id_, {"results": answer(params, s)}))
        elif "request_error" in s:
            send({"jsonrpc": "2.0", "id": id_, "error": rpc_error(s["request_error"])})
        elif "results" in s:
            send({"jsonrpc": "2.0", "id": id_, "result": {"results": s["results"]}})
        else:
            send({"jsonrpc": "2.0", "id": id_, "result": {"results": answer(params, s)}})
    elif method == "shutdown":
        how = script().get("shutdown")
        if how == "garbage":
            out.write(b"no colon in this header\r\n\r\n")
            out.flush()
        elif how == "error":
            send({"jsonrpc": "2.0", "id": id_, "error": rpc_error(-32603)})
        elif how == "exit":
            sys.exit(0)
        elif how == "not_null":
            send({"jsonrpc": "2.0", "id": id_, "result": "done"})
        else:
            send({"jsonrpc": "2.0", "id": id_, "result": None})
    elif method == "exit":
        sys.exit(0)
    elif id_ is not None:
        send({"jsonrpc": "2.0", "id": id_, "error": rpc_error(-32601)})
