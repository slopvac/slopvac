"""Scripted fake plugin for the host conformance tests (H-tests).

Usage: fake_plugin.py MODE [OVERRIDES_JSON] [PIDFILE]

MODE picks a misbehaviour; OVERRIDES_JSON is deep-merged into the initialize
result. With PIDFILE, every message received is appended to PIDFILE.log as a
JSON line before the plugin acts on it, so a test can wait for receipt and
check what the host sent. Standard library only.
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


def record(msg):
    if pidfile:
        with open(pidfile + ".log", "a") as f:
            f.write(json.dumps(msg) + "\n")


def frame(obj):
    body = json.dumps(obj).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def sized_frame(obj, size):
    """Frame obj with an envelope `pad` field so the body is exactly `size` bytes."""
    obj = dict(obj, pad="")
    base = len(json.dumps(obj).encode())
    assert size >= base, (size, base)
    obj["pad"] = "p" * (size - base)
    body = json.dumps(obj).encode()
    assert len(body) == size
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


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
    out.write(frame(obj))
    out.flush()


def send_sized(obj, size):
    out.write(sized_frame(obj, size))
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
        if mode == "count":
            evidence = str(tag_requests)
        if mode in ("ids", "ids-cancel"):
            evidence = str(current_id)
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


def write_pids(path, pids):
    """Write the pids to a temporary name, then rename, so a reader never sees part."""
    with open(path + ".tmp", "w") as f:
        f.write("\n".join(map(str, pids)))
    os.rename(path + ".tmp", path)


def wait_for(path):
    """The contents of a file that a child creates by rename."""
    end = time.time() + 10
    while not os.path.exists(path) and time.time() < end:
        time.sleep(0.01)
    with open(path) as f:
        return f.read().strip()


def spawn_orphan():
    """A child in the same group that keeps the stdio pipes open, then the parent leaves."""
    child = subprocess.Popen(["sleep", "300"]).pid
    if pidfile:
        write_pids(pidfile, [child, os.getpid()])


def spawn_children():
    pids = [subprocess.Popen(["sleep", "300"]).pid]
    pids.append(subprocess.Popen(["sleep", "300"], start_new_session=True).pid)
    if pidfile:
        write_pids(pidfile, pids)


def spawn_nested():
    """A child that leaves the group with `setsid` and starts a child of its own."""
    inner = pidfile + ".inner"
    sh = subprocess.Popen(
        ["sh", "-c", 'sleep 300 & echo $! > "$0.tmp"; mv "$0.tmp" "$0"; wait', inner],
        start_new_session=True,
    ).pid
    write_pids(pidfile, [sh, wait_for(inner)])


# Generation $2 writes its pid to "$1.$2", starts generation $2 - 1 and waits.
# Generation 1 sleeps.
CHAIN = """echo $$ > "$1.$2.tmp"; mv "$1.$2.tmp" "$1.$2"
if [ "$2" -gt 1 ]; then sh -c "$0" "$0" "$1" $(($2 - 1)) & wait; else exec sleep 300; fi"""


def spawn_chain(depth):
    """A chain of `depth` generations outside the group, each a child of the one before."""
    subprocess.Popen(
        ["sh", "-c", CHAIN, CHAIN, pidfile, str(depth)], start_new_session=True
    )
    write_pids(pidfile, [wait_for("%s.%d" % (pidfile, g)) for g in range(depth, 0, -1)])


def move_group():
    """Start a child in a new process group of the session, then move into that group."""
    child = subprocess.Popen(["sleep", "300"], preexec_fn=lambda: os.setpgid(0, 0)).pid
    os.setpgid(0, child)
    write_pids(pidfile, [os.getpid(), child])


if mode == "stray-stdout":
    out.write(b"hello\r\n")
    out.flush()

shut = False
pending_tag = None
tag_requests = 0
current_id = None
sized = int(mode.split(":")[1]) if mode.startswith("body:") else None
init_sized = int(mode.split(":")[1]) if mode.startswith("init-body:") else None
# init-then:N writes the initialize reply and an N-byte notification in one write.
init_then = int(mode.split(":")[1]) if mode.startswith("init-then:") else None
while True:
    msg = read_msg()
    if msg is None:
        sys.exit(0)
    record(msg)
    method = msg.get("method")
    id_ = msg.get("id")
    current_id = id_
    if method == "tagger/tag":
        tag_requests += 1
    if method == "initialize":
        if mode == "silent-init":
            time.sleep(300)
        if mode == "version-unsupported":
            error(id_, -32001, "protocol-version-unsupported")
            continue
        if mode == "hang" and pidfile:
            write_pids(pidfile, [os.getpid()])
        if init_sized:
            send_sized(
                {"jsonrpc": "2.0", "id": id_, "result": init_result()}, init_sized
            )
            continue
        if init_then:
            reply = frame({"jsonrpc": "2.0", "id": id_, "result": init_result()})
            noise = sized_frame({"jsonrpc": "2.0", "method": "noise"}, init_then)
            out.write(reply + noise)
            out.flush()
            continue
        result(id_, init_result())
    elif method == "initialized":
        if mode == "orphan-init":
            spawn_orphan()
            sys.exit(0)
        if mode == "flood":
            for _ in range(1500):
                send({"jsonrpc": "2.0", "method": "noise", "params": {}})
        if mode == "children" or mode == "ignore-shutdown":
            spawn_children()
        if mode == "nested":
            spawn_nested()
        if mode.startswith("chain:"):
            spawn_chain(int(mode.split(":")[1]))
        if mode == "move-group":
            # The leader stops reading, so only a kill ends it.
            move_group()
            time.sleep(300)
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
        if mode == "eof-before-exit":
            # Standard output ends first; the process leaves only after the
            # host has closed standard input, which it does after `exit`, and
            # a further pause. A host that treats this end of stream as a
            # failure has killed the plugin by then; the host's 10 s exit
            # timer leaves the pause no way to fail a correct host.
            os.close(1)
            while inp.read(65536):
                pass
            time.sleep(0.3)
        sys.exit(0 if shut else 1)
    elif method == "$/cancelRequest":
        if mode in ("cancel-ok", "ids-cancel") and pending_tag is not None:
            error(pending_tag, -32005, "request-cancelled")
            pending_tag = None
        elif mode == "cancel-late-answer" and pending_tag is not None:
            result(pending_tag, tags_for(pending_params))
            pending_tag = None
    elif method == "tagger/tag":
        if mode in ("hang", "cancel-ok", "cancel-late-answer"):
            pending_tag = id_
            pending_params = msg["params"]
        elif mode == "orphan-tag":
            spawn_orphan()
            sys.exit(3)
        elif mode == "stderr-tail":
            for i in range(20000):
                sys.stderr.write("line-%05d\n" % i)
            sys.stderr.flush()
            sys.exit(3)
        elif sized:
            send_sized(
                {"jsonrpc": "2.0", "id": id_, "result": tags_for(msg["params"])}, sized
            )
        elif mode == "duplicate-response":
            result(id_, tags_for(msg["params"]))
            result(id_, tags_for(msg["params"]))
        elif mode == "top-array":
            send([])
        elif mode == "ids-cancel" and tag_requests == 1:
            pending_tag = id_
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
    elif method == "extractor/extract":
        # One segment over the first word, converted to upper case.
        res = []
        for doc in msg["params"]["documents"]:
            word = doc["text"].split(" ")[0]
            seg = {"span": {"start": 0, "end": len(word.encode())}, "context": "prose"}
            res.append(
                {"document": doc["id"], "segments": [dict(seg, text=word.upper())]}
            )
        result(id_, {"results": res})
    elif method == "scorer/score":
        # 1.5 per finding; a document without findings gets no score.
        res = []
        for doc in msg["params"]["documents"]:
            entry = {"document": doc["id"]}
            if doc["findings"]:
                entry["score"] = 1.5 * len(doc["findings"])
            res.append(entry)
        result(id_, {"results": res})
    elif method == "reporter/render":
        p = msg["params"]
        out_text = "%s:%s" % (p["report"]["schema_version"], p["options"]["width"])
        result(id_, {"output": out_text, "media_type": "text/plain"})
    elif method == "provider/complete":
        # The text reversed; an item whose instruction is "fail" gets an error.
        p = msg["params"]
        res = []
        for item in p["items"]:
            if item.get("instruction") == "fail":
                err = {
                    "code": -32007,
                    "message": "off",
                    "data": {"slopvac_code": "offline"},
                }
                res.append({"item": item["id"], "error": err})
            else:
                res.append(
                    {
                        "item": item["id"],
                        "text": item["text"][::-1],
                        "model": p["model"] + "-echo",
                    }
                )
        result(id_, {"results": res})
    elif method == "recipe/plan":
        p = msg["params"]
        files = [
            {
                "path": p["harness"] + "/AGENTS.md",
                "action": "write",
                "content": p["options"]["marker"],
            },
            {"path": "old.md", "action": "remove"},
        ]
        result(id_, {"files": files})
    elif id_ is not None:
        error(id_, -32601, "method-not-found")
