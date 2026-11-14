# Slopvac 3.0 plugin protocol v1

Status: contract for 3.0. Bead `slopvac-8l4.2.6`. Plan: [slopvac-3.md](../design/slopvac-3.md). Decision: [0001-slopvac-3-architecture.md](../design/0001-slopvac-3-architecture.md), decisions 8 and 9. Policies: [policies.md](policies.md).

A code plugin is a separate program. The host starts it as a subprocess and speaks JSON-RPC 2.0 to it over the plugin's standard input and output. This document fixes the wire format, the lifecycle, and the hooks. The schema is [`plugin-protocol.schema.json`](../../schemas/v3/plugin-protocol.schema.json). It holds the message shapes and their field descriptions. This document does not repeat them.

2.x has no plugin API. Its only subprocess is Vale, which `vale.py` starts with a timeout and which 3.0 removes (`slopvac-8l4.16.4`). Every statement below is a 3.0 requirement.

"Must" binds the party named in the sentence. The host is the slopvac binary, and the plugin is the subprocess.

## 1. Constraints from the plan

- The host uses no async runtime. It runs one reader thread per plugin, joined to the work threads by `crossbeam-channel`, and enforces deadlines with `recv_deadline` or `select!`. The spike in `slopvac-8l4.2.17` ran this design with three children, one of which hung, in 49 lines.
- The host starts a plugin only after `slopvac plugins trust` has recorded the content hash of the plugin. Nothing in this protocol records or implies trust.
- A plugin that may open connections declares it (section 4.2). The host starts it only when offline mode is off, and policies.md sections 1.5 and 1.6 apply.
- The unknown-field rule of policies.md section 3.3 applies in both directions (section 9).

## 2. Transport

### 2.1 Framing

The framing is the one the Language Server Protocol uses. Each message is a header block, an empty line, and a body:

```text
Content-Length: 87\r\n
\r\n
{"jsonrpc":"2.0", ... }
```

- The body is one JSON value in UTF-8 without a byte order mark. `Content-Length` counts the body bytes.
- `Content-Length` is required. It is a decimal number with no sign and no leading zeros, except for the single digit 0. A reader treats any other value as a framing error.
- A sender may add a `Content-Type` header. A reader accepts and ignores every header other than `Content-Length`. A header line has at most 1024 bytes, and a header block has at most 8 lines.
- Header names compare without regard to case. Lines end in `\r\n`. A bare `\n` is a framing error.
- The body is a JSON object. A top-level array (JSON-RPC batch) is invalid in v1 and gets the error `invalid-request`. Batching happens inside the params of a hook (section 5.3).
- The plugin writes nothing but frames to standard output. Any other byte there is a framing error. Logs go to standard error (section 7).

A framing error is fatal. The host kills the plugin, fails every pending request with `plugin-protocol-violation`, and does not reuse the plugin during the run. A plugin that detects a framing error from the host exits with status 1.

### 2.2 Message size and depth

| Limit | Default | Rule |
|---|---|---|
| `maxMessageBytes` | 16 MiB (16777216) | Body size. The floor is 64 KiB (65536). The host proposes it in `initialize`; the plugin may lower it in its reply, not raise it. The lower value binds both directions. |
| `maxBatchDocuments` | 64 | Documents in one request. The host splits larger batches. |
| JSON nesting depth | 64 | A reader rejects deeper values as a framing error. |
| Header line | 1024 bytes | Section 2.1. |

A sender must not send a body over the limit. A reader checks `Content-Length` before it allocates a buffer, and treats a value over the limit as a framing error. It stops there and does not read the body. The host splits a batch by byte size as well as by document count. One document whose own message would exceed the limit gets the error `message-too-large` for that document, and the host does not send it.

A duplicate key in an object is a framing error in v1, because readers disagree about which value wins.

## 3. Lifecycle

```text
host                                plugin
 |-- spawn ---------------------------->|
 |-- initialize (request) ------------->|
 |<-- initialize (result) --------------|
 |-- initialized (notification) ------->|
 |-- hook requests ... <--> responses ->|
 |-- shutdown (request) --------------->|
 |<-- shutdown (null) ------------------|
 |-- exit (notification) -------------->|
 |                                      | exit status 0
```

1. **Spawn.** The host starts the plugin from the verified, trusted copy (section 11.1). It passes the arguments from the plugin manifest and an environment built from an allowlist. It pipes all three standard streams.
2. **Initialize.** The first message from the host must be the `initialize` request. A plugin that gets another request first answers `not-initialized` (-32003). The reply fixes the protocol version, the hooks, the features and the limits. Section 4 describes the negotiation.
3. **Initialized.** The host then sends the `initialized` notification. A plugin must not expect hook requests before it, and must not send a request of its own. The v1 protocol has no plugin-to-host requests.
4. **Work.** The host sends hook requests (section 5).
5. **Shutdown.** The host sends `shutdown`. The plugin finishes or abandons pending work, answers `null`, and answers every later request except `exit` with `invalid-request`. The host then sends `exit`. The plugin exits with status 0 after `shutdown` and `exit`, and with status 1 after `exit` alone.
6. **End of input.** Closing the plugin's standard input has the effect of `exit`. A plugin that sees end of input exits at once.
7. **Kill.** The host kills a plugin that has not exited 2 seconds after `exit`. It kills the whole process group on Unix and the job object on Windows, so the children of the plugin die too.

A plugin that exits or crashes mid-run fails all its pending requests with `plugin-exited`. The host reports the exit status and the tail of standard error (section 7). It does not restart the plugin in the same run. The affected operation is incomplete, and the run exits 2.

The host sends `shutdown` and `exit` when the run ends, including after an error. No plugin outlives the host. The host uses a drop guard and a signal handler so that a Ctrl-C kills the process groups.

## 4. Capability negotiation

### 4.1 Versions

`InitializeParams` carries `protocolVersion`, the highest version the host speaks, and `minProtocolVersion`, the lowest. A missing minimum means 1. This follows policies.md section 3.4 and adds the lower bound that makes a range.

The plugin picks the highest version both sides support and returns it as `protocolVersion` in `InitializeResult`. The host checks that the value lies in its own range. When the ranges do not overlap, the plugin answers `protocol-version-unsupported` (-32001) with its own range in `data.supported`. The host then does not load the plugin and prints both ranges.

A version is a major version in the sense of policies.md section 3.6. A change within version 1 is additive. Version 1 is the only version in 3.0.

### 4.2 Hooks and features

- The host lists in `capabilities.hooks` the hook kinds it can call. The plugin lists in its reply each hook it implements as a kind and an id. The host calls only declared hooks, and only with a kind in its own list. A hook kind that the host does not know is ignored.
- A hook id is local to the plugin and to its kind. The host qualifies it as `<plugin>:<kind>/<id>` in diagnostics.
- A feature is an optional capability with a name. The host lists the features it offers. The plugin splits the features it wants into `required` and `optional`.
- A required feature that the host lacks stops the load. The host reports `required-feature-missing` (-32002) with the feature name. A missing optional feature is skipped without notice. The plugin finds out from the host's list in `initialize`.
- The registry of v1 holds one feature, `document-path`. With it granted, each `Document` carries a `path` (section 8). Without it the plugin sees no path.
- A name with the prefix `x-` is a private extension. Two parties may agree on one. Everyone else ignores it. A name that is neither registered nor prefixed is ignored as unknown, and as a required feature it stops the load.
- `requiresNetwork` tells the host that the plugin may open network connections. The host does not start such a plugin when `offline` is true in `initialize` or in sticky offline mode, and prints `offline: <plugin> needs the network` (exit 2) when the user asked for it. A plugin that declares `requiresNetwork: false` and connects anyway breaks the contract. The host cannot detect that in 3.0 (section 11.2).

## 5. Hooks

### 5.1 Methods

Each hook kind has one method. The host sends it as a request. The schema holds the parameter and result shapes under the `$defs` below.

| Kind | Method | Params | Result | Purpose |
|---|---|---|---|---|
| tagger | `tagger/tag` | [`TagParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/TagParams) | [`TagResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/TagResult) | Context tags with a span, a confidence and evidence. |
| extractor | `extractor/extract` | [`ExtractParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/ExtractParams) | [`ExtractResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/ExtractResult) | Split a format such as AsciiDoc into prose segments. |
| reporter | `reporter/render` | [`RenderParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/RenderParams) | [`RenderResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/RenderResult) | Render one lint report as text. |
| fixer | `fixer/fix` | [`FixParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/FixParams) | [`FixResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/FixResult) | Propose edits for findings. |
| scorer | `scorer/score` | [`ScoreParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/ScoreParams) | [`ScoreResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/ScoreResult) | Compute a score per document. |
| provider | `provider/complete` | [`CompleteParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/CompleteParams) | [`CompleteResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/CompleteResult) | Ask a model for a review. |
| recipe | `recipe/plan` | [`RecipeParams`](../../schemas/v3/plugin-protocol.schema.json#/$defs/RecipeParams) | [`RecipeResult`](../../schemas/v3/plugin-protocol.schema.json#/$defs/RecipeResult) | Plan the files that set up an agent harness. |

The lifecycle methods are `initialize`, `initialized`, `shutdown`, `exit` and `$/cancelRequest`. A method with the prefix `$/` is optional protocol traffic. A reader drops an unknown `$/` notification and answers an unknown `$/` request with `method-not-found`. Any other unknown notification is dropped, and an unknown request gets `method-not-found` (-32601).

### 5.2 Semantics that every hook shares

- **Hooks are pure.** A hook reads the request and returns a result. It may keep a cache, but its result for a request must not depend on earlier requests. The host may retry or reorder requests, or skip them on a cache hit.
- **Taggers, extractors, fixers and scorers are deterministic.** The same request gets the same result. Run-to-run differences would break the golden suite and the cache. A provider is the one exception.
- **A provider is the only hook that may use the network.** Slopvac core calls a provider only from `slopvac review`, and `lint` leaves every provider idle. The call needs a plugin that the user trusted and named under `[review] provider`. The host sends the text the review type needs and no more (policies.md section 1.5). The `model` in the result is the name the provider reports, and the host compares it with the author model.
- **A recipe writes nothing.** It returns a plan. The host checks each path, shows the plan, and applies it only when the command allows writes (section 8.2).
- **A reporter returns text.** It does not write files. The host strips terminal control characters, other than newline and tab, before it prints to a terminal.

### 5.3 Batching

Every hook that works on documents takes a `documents` array. The host sends all the documents of one request in one message, up to `maxBatchDocuments` and `maxMessageBytes`. A result holds exactly one entry per document, and each entry echoes the document `id`. Order does not matter, because the host matches by id. A missing or extra entry, or a duplicate id, makes the whole result invalid (`plugin-output-invalid`).

A document entry may carry `error`. That fails the one document, and the run exits 2 for it. Other documents in the batch stand. A request-level error fails the whole batch.

Each request names one hook id, so a batch holds documents for a single hook.

### 5.4 Positions

Spans use the `Span` def in `common.schema.json`. They are half-open byte ranges into the UTF-8 `text` of the document in the same request. A span must lie inside the text and on character boundaries. The host drops a span that does not. It counts that as `plugin-output-invalid`, which makes the run incomplete. Semantics for line and column conversion belong to `slopvac-8l4.2.11`. A plugin reports spans in bytes.

A segment from an extractor keeps the span of the source it came from. When the converted `text` differs from the source, a finding inside it maps to the whole segment span. Finer mapping is outside v1.

## 6. Requests in flight, cancellation and timeouts

### 6.1 Concurrency

- Request ids are positive integers, unique among the in-flight requests of one sender. The host counts up from 1.
- A plugin states `maxInFlight` in its reply. The host keeps at most that many requests pending. The default plugin value in the Python SDK is 1.
- A plugin may answer in any order.
- A response with an id that matches no pending request is ignored and logged. Two responses to one id: the second is ignored.

### 6.2 Cancellation

The host sends `$/cancelRequest` with the `id` it does not want. The plugin should stop work on that request and answer it, with the result or with `request-cancelled` (-32005). The host discards either answer. The timeout rule covers a plugin that does not reply.

Cancellation is advisory. A cancel for an unknown or finished id is ignored. The host sends a cancel when a deadline passes, when the user interrupts the run, and when another failure makes the result useless.

### 6.3 Timeouts

The host owns every timeout. The plugin has no way to ask for more time.

| Timer | Default | On expiry |
|---|---|---|
| `initialize` reply | 10 s | The host kills the plugin and reports `plugin-timeout`. |
| Hook request | 30 s | The host sends `$/cancelRequest`. |
| `provider/complete` | 120 s | Same. |
| Cancel grace | 2 s | The host kills the plugin if it has not answered the cancelled request. It reports `plugin-timeout`. |
| `shutdown` reply | 3 s | The host sends `exit` and goes on to the kill rule. |
| Exit after `exit` | 2 s | The host kills the process group. |

The config can change the hook timeouts for each plugin. The config bead owns that key (`slopvac-8l4.4.2`). A value of 0 is invalid. A plugin killed by a timeout is not reused in the run, and its pending requests fail with `plugin-timeout`. A run with a timeout exits 2. A timeout never changes a lint verdict into a pass.

## 7. Standard error

- The host reads standard error on its own thread from the moment of spawn. A full pipe would stall the plugin, so the host never leaves the stream unread.
- The host keeps the last 64 KiB in a ring buffer and discards the rest. It treats the bytes as untrusted text.
- Standard error is a log for people. It carries no protocol traffic, and the host never parses it.
- The host shows it in two cases.
  - Under `SLOPVAC_LOG` it logs each line at debug level, under the logging rules of policies.md section 1.7.
  - After a plugin fails, it shows the last 4 KiB in the error and replaces control characters other than newline and tab.
- A plugin must not write document text, finding text or secrets to standard error. The host cannot redact free text, so the rule rests on the plugin.

## 8. Paths and files

### 8.1 Paths the plugin sees

A plugin sees no path unless it required or asked for `document-path` and the host granted it. A granted `path` is relative to the project root. For standard input it is `<stdin>`, and for a file outside the root it is `<external>/<file name>`, as in policies.md section 1.7. It is relative, and has no `..` segment, no drive letter and no backslash. The host omits the project root, the home directory and the cache directory.

The working directory of the plugin is its own installed directory, not the project.

### 8.2 Paths the plugin returns

Only recipes return paths. Each is relative to the project root with the same restrictions. The host also resolves symlinks and refuses any path that leaves the root, and refuses to write through a symlink that points out of it. Schema patterns are a first filter and not a boundary. The host check is the boundary.

## 9. Compatibility rules

- A reader ignores a field it does not know, in every message and at every depth. It may log the field at debug level.
- A key with the prefix `x-` is ignored everywhere. Its meaning stays undefined. A plugin or host may use one for private data.
- The schema leaves `additionalProperties` open on every protocol message. This differs from user-written files, where an unknown key is an error (policies.md section 3.3).
- A reader ignores an enum value it does not know only where the schema marks the enum as extensible. In v1 those are hook kinds and feature names. Closed enums such as `task`, `action` and `severity` stay closed, so a new value needs a new protocol version.
- A required field that is missing or has the wrong type makes the message invalid, however lenient the rest of the rules are.
- Additive changes within version 1 are: a new optional field, a new hook kind, a new feature, a new optional notification. Everything else needs version 2.

## 10. Error codes

Errors use the JSON-RPC error object. `code` is numeric. `data.slopvac_code` is the stable kebab-case string that policies.md section 3.5 requires, and the schema requires it on every error a plugin sends. A numeric code never changes meaning. Messages are not an interface.

| Code | `slopvac_code` | Meaning |
|---|---|---|
| -32700 | `parse-error` | The body is not valid JSON. |
| -32600 | `invalid-request` | The body is not a valid message, or it is a batch array, or the plugin is shut down. |
| -32601 | `method-not-found` | Unknown method, or a hook the plugin did not declare. |
| -32602 | `invalid-params` | Params do not match the schema, or name an unknown hook id. |
| -32603 | `internal-error` | The plugin failed. |
| -32001 | `protocol-version-unsupported` | No common version. `data.supported` holds the plugin range. |
| -32002 | `required-feature-missing` | The host lacks a feature the plugin requires. |
| -32003 | `not-initialized` | A request arrived before `initialize` finished. |
| -32004 | `message-too-large` | A message exceeds `maxMessageBytes`. |
| -32005 | `request-cancelled` | The plugin stopped a request after a cancel. |
| -32006 | `document-invalid` | The plugin cannot process this document, for example binary content. |
| -32007 | `offline` | A provider refused the call because offline mode is on. |

The range -32099 to -32000 is the slopvac range. The plugin may add codes from -32050 to -32099 for its own use if it also sets `data.slopvac_code`, and the host treats them as `internal-error`. The host reserves the rest of the range.

The host raises these codes in its own diagnostics. They have no numeric code, because no message carries them:

| `slopvac_code` | Cause |
|---|---|
| `plugin-untrusted` | The content hash has no recorded trust. The message prints the `slopvac plugins trust` command. |
| `plugin-hash-mismatch` | The installed bytes differ from the hash that trust recorded. |
| `plugin-timeout` | A timer in section 6.3 expired. |
| `plugin-exited` | The process ended with requests pending. |
| `plugin-protocol-violation` | A framing error, a limit breach or a duplicate key. |
| `plugin-output-invalid` | A result does not match its schema, or breaks a rule such as span bounds. |
| `plugin-spawn-failed` | The program did not start. |

Exit codes stay as policies.md section 3.5 defines them. Any plugin failure makes the affected run exit 2, never 0, and never 1 unless findings fail the gate.

## 11. Security

The following hand-offs go to the threat-model bead. The threat model and independent review in `slopvac-8l4.9.11` set the final controls. The list says what the protocol exposes and what the host design has to settle. Items marked "3.0 gap" have no control yet.

### 11.1 What the protocol fixes

- The trust check runs before spawn. Trust is a content hash recorded by `slopvac plugins trust`. No message, flag or prompt records it.
- The plugin runs from a verified copy. The host hashes the files, then runs them from a read-only, content-addressed directory, so a swap between check and use does not apply.
- The environment is an allowlist: `PATH` set by the host, a locale, and variables the plugin manifest names and the user allowed. No tokens, no `HOME`, no CI variables. A provider that needs a key gets it by an explicit config entry.
- All file descriptors other than the three standard streams are closed in the child.
- The host sends only document text, findings and options. Paths, when granted, are relative.
- The host validates every result before use and never executes any part of it.

### 11.2 Hand-offs to `slopvac-8l4.9.11`

| Area | Question for the threat model | Starting position |
|---|---|---|
| Resource limits | CPU time, memory, process count, open files and disk use of a plugin. | 3.0 gap. The protocol sets timeouts and sizes only. Candidate controls: `setrlimit` on Unix, job objects on Windows, a wall-clock cap per run, a cap on total output bytes per plugin. |
| Subprocess containment | Whether a plugin can read the user's files, call the network, or start further programs. | 3.0 gap. Plugins run with the user's rights. policies.md section 1.2 records the network risk. Candidates: macOS `sandbox-exec`, Linux Landlock and seccomp, Windows AppContainer. The kill path covers process groups only. |
| Message size and shape | Memory exhaustion by a large body, deep nesting, many small messages, a flood of responses or of standard error. | Section 2.2 limits size, depth and header size. The threat model sets the per-plugin message rate and the total bytes a run accepts. |
| Path exposure | Whether `document-path` leaks names, and whether recipe paths can write outside the project. | Sections 8.1 and 8.2. The model should check Windows path forms, case-insensitive file systems, and symlink and rename races during `recipe/plan` application. |
| Trust scope | What the hash covers: the entry file only, or the whole directory, interpreter and dependencies. Python plugins run in their own environment. | The hash must cover everything that executes. Open for Python environments built at install time. |
| Output as input | Terminal escape injection through a reporter or an error, and Markdown or HTML injection in reports. | The host strips control characters from terminal output (sections 5.2 and 7). The model reviews the other formats. |
| Provider data flow | What leaves the machine, and whether a plugin can read secrets from the environment. | policies.md section 1.5 limits the text. The allowlist in section 11.1 limits the environment. The plugin can still send any text it receives. |
| Malicious plugin | A plugin that lies about `requiresNetwork`, reports false spans, or stalls. | Spans are checked, stalls hit timeouts. The false declaration is a 3.0 gap. |
| Duplicate keys and parser differences | Ambiguity between the host parser and the plugin parser. | Section 2.2 rejects duplicate keys. |
| Log leakage | Plugin standard error in host logs. | Section 7. The host cannot redact free text. |

## 12. Conformance test outline

The schema and the instances under `schemas/v3/conformance/plugin-protocol/` check message shapes. The tests below check behavior. The host suite belongs to `slopvac-8l4.9.6` and the plugin suite to `slopvac-8l4.9.7`. Policies.md tests V3, V4, N8 and N12 depend on the rows marked with them.

A host passes when it satisfies every H test against a scripted fake plugin. A plugin passes when it satisfies every P test against a scripted fake host. Both suites replay the schema instances first and treat any message that fails the schema as a failure.

### 12.1 Host

| ID | Setup | Expected |
|---|---|---|
| H1 | Plugin sends a body of exactly `maxMessageBytes`, then one byte more. | The first passes. The second is a `plugin-protocol-violation` and a kill, before the body is read. |
| H2 | Frames with a bad `Content-Length` (sign, hex, leading zero, missing), a bare `\n`, a long header line. | Each is a framing error. |
| H3 | Stray text on plugin standard output before a frame. | Framing error and kill. |
| H4 | Plugin stays silent after `initialize`. | Kill after 10 s, `plugin-timeout`. |
| H5 | Plugin returns a version outside the host range. | Not loaded, both ranges printed. (V4) |
| H6 | Plugin requires an unknown feature, or lists an unknown `x-` feature as optional. | The first stops the load, naming it. The second loads. (V4) |
| H7 | Plugin declares a hook kind the host does not know. | Ignored. The known hooks work. |
| H8 | The config names a hook the plugin did not declare. | The host reports the config error and skips the hook. |
| H9 | Result with unknown fields and `x-` keys at every depth. | Accepted, extras ignored. (V3) |
| H10 | Result with a missing document, an extra one, a duplicate id. | `plugin-output-invalid`, whole result rejected. |
| H11 | Result with a span past the text end, or inside a multi-byte character. | `plugin-output-invalid`. |
| H12 | Per-document `error` in a batch of three. | The other two stand. The run exits 2. |
| H13 | Plugin takes longer than the hook timeout, then ignores `$/cancelRequest`. | Cancel sent, kill after the 2 s grace, `plugin-timeout`, plugin not reused. |
| H14 | Plugin answers a cancel with `request-cancelled`. | The answer is discarded. No error is reported. |
| H15 | Plugin exits mid-request. | Pending requests fail with `plugin-exited`. The exit status and stderr tail are shown. No restart. |
| H16 | Plugin writes 10 MiB to standard error. | The host keeps reading. Only the last 64 KiB is kept. |
| H17 | Plugin ignores `shutdown`. | `exit` after 3 s, kill 2 s later, the process group is gone. |
| H18 | The run is interrupted by a signal. | Process groups are killed. |
| H19 | `requiresNetwork: true` with offline mode on. | The plugin does not start. Message `offline: <plugin> needs the network`. (N12) |
| H20 | An untrusted plugin, and a plugin whose files changed after trust. | No spawn. The error names `slopvac plugins trust`. (N8) |
| H21 | A document with a path outside the root, and `document-path` granted. | The plugin sees `<external>/<file name>`. |
| H22 | A recipe returns an absolute path, a `..` path, and a symlink target outside the root. | All three are refused. Nothing is written. |
| H23 | Two documents whose batch exceeds `maxMessageBytes`. | The host sends two messages. |
| H24 | A response with an unknown id, and a second response to a finished id. | Ignored and logged. |
| H25 | A top-level array, a duplicate key, a nesting depth of 65. | Each is a framing error. |

### 12.2 Plugin

| ID | Setup | Expected |
|---|---|---|
| P1 | Host sends `initialize` with a range that includes 1. | The reply lists hooks, features, limits and `protocolVersion` 1, and validates against `InitializeResult`. |
| P2 | Range 2 to 3. | `protocol-version-unsupported` with `data.supported`. |
| P3 | A request before `initialize`. | `not-initialized`. |
| P4 | Unknown method as request, and as notification. | `method-not-found`, and silence. |
| P5 | Params with unknown fields, `x-` keys, and unknown hook kinds in the host's list. | Processed normally. |
| P6 | A request for a hook id the plugin never declared. | `invalid-params`. |
| P7 | A batch with a document of invalid content. | One entry has `error`. The others stand. |
| P8 | The same request twice. | Equal results (pure and deterministic hooks). |
| P9 | `$/cancelRequest` for a running request, and for an unknown id. | Answer or `request-cancelled` within the grace period, and silence. |
| P10 | `shutdown`, then a request, then `exit`. | `null`, `invalid-request`, exit status 0. |
| P11 | `exit` with no `shutdown`. | Exit status 1. |
| P12 | End of input. | Exit at once. |
| P13 | A body at `maxMessageBytes` plus one. | The plugin does not read it as a message. It exits with status 1 and no partial output. |
| P14 | A run that logs a lot. | Standard output holds frames only. |
| P15 | Every non-error result. | Validates against its `$defs`. Spans are in range, on boundaries. |
| P16 | `offline: true` in `initialize` with a provider plugin. | The provider answers `offline` (-32007) and opens no connection. |
| P17 | A recipe plan. | Relative paths only. No write by the plugin. |

## 13. Owners

| Topic | Bead |
|---|---|
| Host: threads, framing, timeouts, kill | `slopvac-8l4.9.6` |
| Python SDK and the plugin conformance suite | `slopvac-8l4.9.7` |
| Trust store and `plugins trust` | `slopvac-8l4.9.4` |
| Schema publication from Rust types | `slopvac-8l4.19.1` |
| Generated plugin docs | `slopvac-8l4.19.5` |
| Span and edit semantics | `slopvac-8l4.2.11` |
| Threat model and review | `slopvac-8l4.9.11` |
| Config keys for timeouts and the provider | `slopvac-8l4.4.2` |
