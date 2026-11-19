# slopvac-plugin

A Python SDK for slopvac plugin protocol v1, and a conformance suite that tests plugins written in any language. The protocol is in [`docs/contract/plugin-protocol.md`](../../docs/contract/plugin-protocol.md).

The SDK uses only the standard library. The conformance suite needs `jsonschema`, which the `conformance` extra installs.

## Write a plugin

Register one handler for each hook, then call `run()`:

```python
from slopvac_plugin import Document, Plugin, Tag

plugin = Plugin("my-tagger", "0.1.0")


@plugin.tagger("main")
def tag(doc: Document, ctx) -> list[Tag]:
    return [Tag("prose", doc.span(0, len(doc.text)), 1.0, "whole document")]


if __name__ == "__main__":
    plugin.run()
```

The decorators are `tagger`, `extractor`, `reporter`, `fixer`, `scorer`, `provider` and `recipe`. Their docstrings give each handler signature. Document hooks receive one `Document` at a time. The SDK batches, echoes document ids, and turns `DocumentInvalid` into a per-document error.

What the SDK does for you:

- **Framing.** It reads and writes `Content-Length` frames. It rejects an oversized body before it reads the body, and then exits with status 1. It rejects duplicate keys, nesting over 64 levels, and integers outside ±(2^53-1).
- **Lifecycle.** It answers `initialize`, `shutdown` and `exit`, and builds the capability reply from the registered hooks. `maxInFlight` defaults to 1.
- **Cancellation.** The host's `$/cancelRequest` sets `ctx.cancelled`. Call `ctx.raise_if_cancelled()` in long loops.
- **Standard output.** `run()` points `print`, logging and descriptor 1 at standard error. Only frames reach the real standard output.
- **Offline mode.** When the host sets `offline`, the SDK answers a provider request with `offline` (-32007) and skips the handler.
- **Output checks.** Before it sends a result, it checks that spans are in range and on character boundaries, that fixer edits lie inside the span of the finding they name, and that recipe paths are relative. A violation becomes `internal-error`, and the SDK logs the cause without document text.

Spans are UTF-8 byte offsets. `Document.span(start, end)` converts character offsets, and `Document.data` holds the bytes. It raises `ValueError` for offsets that [the position contract](../../docs/contract/positions.md) calls invalid: outside the text, a start after the end, or between the CR and LF of one CRLF.

`Plugin(network=...)` must equal the `network` value in the plugin manifest. The hooks you register must be a subset of the hooks the manifest declares.

## Run the conformance suite

```sh
pip install "slopvac-plugin[conformance]"
slopvac-plugin-conformance --manifest slopvac-plugin.toml -- python plugin.py
```

`python -m slopvac_plugin.conformance` runs the same command. The suite starts the plugin command, drives cases P1 to P19 from section 12.2 of the contract over standard input and output, and prints a report. The plugin can use any language. The suite checks every message the plugin sends against `plugin-protocol.schema.json`.

| Option | Meaning |
|---|---|
| `--manifest FILE` | Plugin manifest (TOML) or declaration (JSON). Enables P19. |
| `--inputs FILE` | JSON with sample requests for each hook kind. |
| `--only P1,P13` | Run only these cases. |
| `--timeout`, `--exit-timeout` | Seconds to wait for a reply and for an exit. |
| `--cancel-grace` | Seconds a plugin may take to answer after a cancel. The default is 2, from section 6.3. |
| `--case-budget`, `--suite-budget` | Wall-clock seconds one case and the whole run may take, all waits included. The defaults are 120 and 900. A case that runs over fails. |
| `--env NAME` | Pass this environment variable to the plugin. Repeat for more names. |
| `--format json` | Machine-readable report. |

The suite treats the plugin as untrusted. It gets only `PATH` and `LANG` (and the variables Windows needs to start a process), plus the names you give with `--env`, so CI tokens and credentials stay out of it. The plugin runs in its own process group, and the suite ends the whole group when it closes the session. The suite also caps how much the plugin may print, ends a plugin that stops reading its input, and checks each reply against the 1 MiB limit before `initialize` and the negotiated `maxMessageBytes` after it. A plugin that declares more than 1,024 hooks, or a feature list longer than 1,024 entries, fails P1. The protocol sets no count, so the suite picks this bound to keep its own work linear. The suite also fails a plugin that sends any call, because protocol v1 has no plugin-to-host calls, and it does so before it validates the call's params.

The case budget covers cleanup too:

- Every wait to close a session stops at the deadline. The suite kills the process group after the last wait.
- A case that ends past its deadline fails, even when its checks passed or it skipped.
- After the plugin exits, the first frame on its output fails P13 or P14 at once.
- On POSIX, a descendant that calls `setsid` leaves the process group, and the suite cannot kill it. When such a process keeps the plugin's output open, the case fails and the suite detaches the pipe. After one more write from that process, its writes fail with a broken pipe. A silent process stays alive: this gap is a known limit of the suite. On Windows, the job object ends every descendant.

Exit 0 means every applicable case passed. Exit 1 means a case failed. Exit 2 means the suite could not run, for example because the command does not start. A skipped case prints its reason and does not fail the run.

### Sample requests

The suite sends built-in documents, findings and recipe options. To test with your own, pass `--inputs`:

```json
{
  "tagger": {
    "documents": [{"id": "a", "text": "Some text.\n"}],
    "invalid_document": {"id": "bad", "text": "bad\u0000bytes"}
  },
  "provider": {}
}
```

- Keys under a hook kind replace the matching request params.
- `invalid_document` turns on P7. The suite sends it with a valid document and expects an error for it alone.
- A `provider` key lets the suite call the provider hook, which may reach a model. Without it, only P16 touches a provider, and only with offline mode on.

### What the cases cannot see

- P16 checks the `offline` error. It cannot observe whether the plugin opened a socket.
- P17 compares the working directory before and after `recipe/plan`. A plugin that writes elsewhere is not caught.
- P9 cancels a request right after it sends it. A fast plugin answers first, which the case allows.

## Example

[`examples/fence-tagger`](examples/fence-tagger) is a tagger built on the SDK with a plugin manifest. Run the suite against it from that directory:

```sh
slopvac-plugin-conformance --manifest slopvac-plugin.toml --inputs conformance-inputs.json -- python plugin.py
```

It passes P1 to P15 and P19. P16 to P18 skip because it declares no provider, recipe or fixer.
