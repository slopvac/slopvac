# Slopvac 3.0 threat model: plugins, trust, OCI and signing keys

Status: contract for 3.0. Bead `slopvac-8l4.9.11`. Plan: [slopvac-3.md](../design/slopvac-3.md). Decision: [0001-slopvac-3-architecture.md](../design/0001-slopvac-3-architecture.md), decisions 8 and 9. Related contracts: [plugin-protocol.md](plugin-protocol.md) and [policies.md](policies.md).

This document models the 3.0 supply chain and plugin runtime. It covers what slopvac fetches, verifies, extracts, trusts and runs. It sets the controls the owning beads must build, and the tests that prove each control. The independent review in `slopvac-8l4.9.11` signs off against this text. `slopvac-8l4.9.2` (OCI client) and `slopvac-8l4.9.6` (plugin host) do not close before that sign-off.

"Must" binds the component named in the sentence. Numbers marked "proposed" are starting values. A bead may change one only with a reason recorded in this file.

## 1. Scope and method

In scope:

- the plugin host and the plugin protocol v1;
- the trust store, the lockfile and the `plugins` commands;
- the OCI client, registry credentials and credential helpers;
- archive extraction and the content-addressed cache;
- managed Python environments for Python plugins;
- network sources: OCI, release archives, git pins, local folders and the plugin index;
- minisign signatures, publisher keys, rotation and revocation;
- regex resource limits in data plugins;
- plugin text that the host prints, renders or writes.

Out of scope for 3.0:

- sandboxing of plugin processes (section 9, R1);
- Sigstore or transparency logs (backlog, decision 9);
- the Vale subprocess of 2.x, which 3.0 removes;
- native Windows: out of scope for 3.0 (no native Windows build), tracked in slopvac-8l4.18.8. WSL is covered by the Linux build; projects on `/mnt` keep the cross-platform path refusals (O9, E6).
- the security of the registry operator, beyond what signatures and digests cover.

Method. Section 2 lists assets, section 3 actors, and section 4 boundaries and flows. Section 5 fixes four design decisions that the threats depend on. Section 6 is the register. Each threat has a STRIDE category, a CWE, a likelihood and an impact (L, M or H), a mitigation, an owning bead and a proving test. Section 7 maps the hand-offs from the plugin protocol (section 11.2) and the acceptance list onto the register. Section 8 lists the beads that carry the new work. Section 9 lists residual risks. Likelihood assumes a published plugin ecosystem. Impact assumes the user's own rights.

## 2. Assets

| ID | Asset | Why it matters |
|---|---|---|
| A1 | The user's account, files and credentials | A code plugin runs with the user's rights. |
| A2 | Registry and provider credentials | Tokens for GHCR or a model service can publish or spend. |
| A3 | The trust store | It decides which code runs. |
| A4 | The lockfile `.slopvac/plugins.lock` | It pins what a project loads. |
| A5 | The publisher keys and the key policy | They decide who may speak for a plugin. |
| A6 | The content-addressed cache and managed environments | The host runs code from here. |
| A7 | Lint verdicts and exit codes | A gate that a plugin can pass or fail on its own is worthless. |
| A8 | Document text sent to plugins and providers | It may hold unpublished or private content. |
| A9 | Project files that agents or CI trust | Hooks, workflows, agent config and `.env` files. |
| A10 | Host availability | CPU, memory, disk, descriptors and processes. |
| A11 | Terminal, report and CI log integrity | Output is read by people and by agents. |

## 3. Actors

| ID | Actor | Capability |
|---|---|---|
| T1 | Malicious plugin publisher | Publishes a valid signed artifact with hostile content. |
| T2 | Registry or network attacker | Controls responses, redirects or the index in transit or at rest. |
| T3 | Compromised publisher account or key | Publishes a hostile update under a real identity. |
| T4 | Hostile pull request author | Changes the lockfile, config, workflows or documents in a project. |
| T5 | Hostile document author | Writes text that a plugin or provider reads. |
| T6 | Local process of the same user | Writes to the cache, the store or the project. |
| T7 | Trusted plugin that turns hostile after an update | Has trust for an earlier hash. |
| T8 | Malicious or confused provider output | Text that came from a model. |

A local attacker with the user's full rights (T6) is only partly in scope. Slopvac cannot defend a store that the attacker may write. The model records where the design still raises the cost (section 9, R6).

This includes a process that writes to a destination directory, its parent or a temporary file's directory while a `slopvac-fs` operation runs. Such a process can swap the destination or its parent, or replace a temporary name. `slopvac-fs` compares the staged file with its name before the rename and fails the write on a mismatch. This narrows the window, so a replaced temporary file fails closed, but it does not close the window (R6).

## 4. Trust boundaries and flows

```text
 B1 registry / network                    B3 user machine, slopvac process
 +--------------------+     F1,F2       +------------------------------------------+
 | OCI registry       |---------------->| OCI client --> verify --> extract --> A6 |
 | plugin index       |  F3 (index)     |                    |                      |
 | release archives   |  F4 (archive)   |   lockfile (A4) <--+                      |
 | git remotes        |  F5 (git)       |                    v                      |
 +--------------------+                 |  trust store (A3) --> plugin host         |
                                        |                         |  F6 stdio        |
 B2 credential helpers  F7 -----------> |                         v                  |
 (docker-credential-*)                  |            B4 plugin process (user rights) |
                                        |             |  F8 files, F9 network, F10   |
 B5 project files (untrusted input)---->|             |  provider calls             |
   F11 documents, config, lockfile      +-------------+------------------------------+
```

| Flow | From, to | Data |
|---|---|---|
| F1 | Registry to OCI client | Auth challenge, token, manifest, blobs, redirects to a CDN host. |
| F2 | OCI client to registry | Credentials, push of layers and manifest (`plugins publish`). |
| F3 | Index to `plugins search` and key lookup | Plugin names, descriptions, publisher keys. A job rebuilds it automatically. |
| F4 | Release host to `plugins sync` | An archive, checked against the lockfile digest. |
| F5 | Git remote to `plugins sync` | A clone at a pinned commit. |
| F6 | Host to plugin and back | Framed JSON-RPC over three pipes (plugin-protocol.md). |
| F7 | Credential helper to the OCI client | Registry credentials on a helper's standard output. |
| F8 | Plugin to the file system | Anything the user may touch. |
| F9 | Plugin to the network | Anything. 3.0 has no enforcement. |
| F10 | Host to a provider plugin | Document text for a review. |
| F11 | Project files to slopvac | Documents, `slopvac.toml`, `.slopvac/plugins.lock`, recipe targets. |

Boundaries. B1 is hostile. B2 is a program chosen by a file in the user's home directory. B3 is trusted. B4 is trusted only as far as its author (section 9, R1). B5 is hostile: a project is data from whoever can open a pull request.

## 5. Design decisions the threats depend on

### 5.1 Signature subject

A minisign signature covers one byte string, the **signed statement**. A layer cannot sign the manifest that contains it, because the layer digest would have to appear in the manifest it signs. The signature therefore does not sign the manifest. It signs a statement that names the manifest's other content, and the verifier recomputes the match.

Layout in the OCI artifact (implemented in `crates/slopvac-plugins/src/sign.rs`):

- The artifact is an OCI manifest with an `artifactType` for the plugin kind, one config blob and the content layers. Two further layers carry the signature, and the verifier finds each by media type:
  - `application/vnd.slopvac.signature.statement.v1+json` holds the canonical statement;
  - `application/vnd.slopvac.signature.minisig.v1` holds the `.minisig` text, at most 4 KiB.
- An artifact has exactly one layer of each type. A missing layer is `MissingSignature` and a repeated one is `DuplicateSignature`. The statement does not list the two signature layers, because the digest of the statement layer depends on the statement.
- The manifest carries the annotations `dev.slopvac.id` and `dev.slopvac.version`. They are trusted only because the statement repeats the same values.

What is signed. The signed bytes are the canonical statement, byte for byte, and nothing else. Minisign signs them in the prehashed mode. The verifier refuses a legacy (non-prehashed) signature.

- The statement is a canonical JSON object of at most 64 KiB, with sorted keys at every depth and no insignificant whitespace.
- The verifier parses it strictly (duplicate keys fail), encodes it again and requires equal bytes, so the signed bytes cannot differ from the parsed value. Unknown fields fail.
- Its fields are:
  - `schema_version`, always 1;
  - `id`, the plugin id from its manifest, and `namespace`, the owner segment of the repository the artifact is published under (both lowercase names, and neither of them `core`);
  - `version`, a semantic version without build metadata;
  - `artifactType`;
  - `config`, the digest of the config blob;
  - `layers`, the digest, size and media type of every layer except the two signature layers, in manifest order;
  - `keyId`, 16 uppercase hex digits;
  - `notBefore`, a time as `YYYY-MM-DDTHH:MM:SSZ` in UTC.

The verifier runs these steps in order, and the first failure ends the check:

1. The statement is at most 64 KiB, strict and canonical, and its fields parse.
2. The key comes from the policy for the statement's namespace (section 5.2) and has the statement's `keyId`.
3. The minisign signature verifies over the exact statement bytes.
4. The statement and the manifest agree. `schema_version` is 1, the listed layers equal the manifest's other layers, and the config digest matches. The `artifactType` matches the manifest and the request. The `id`, `namespace` and `version` match the request, and the annotations match.
5. `notBefore` is a real calendar time, and the rollback rule of section 5.3 holds.

A layer that the statement does not list makes the artifact invalid. The minisign trusted comment is signed, but the host does not use it to decide anything.

After verification the lockfile pins the **manifest digest**, and records the `keyId` and `notBefore` of the signature in its `[plugins.signature]` table. `sync` fetches by that digest, never by tag, and verifies the signature on every run. It fails with `SignatureDiffersFromLock` when the key id or `notBefore` differs from the lock. A registry cannot change what a pinned digest resolves to. Referrers are not used. GHCR lacks the referrers API (`slopvac-8l4.2.17`), and the layer approach works everywhere.

What the signature does not cover: the registry tag (a tag is mutable and only the digest counts), anything outside the artifact, and the behavior of a signed plugin. A signature says who published the bytes. It says nothing about whether they are safe.

### 5.2 Publisher-key authority

The plugin index is rebuilt by a job. Whoever controls that job, or a pull request that the job accepts, controls the index. If the index were the only place that binds a plugin to a key, an index compromise would let an attacker publish a validly signed artifact under any name. So the index is not an authority for keys.

- Two sources supply keys, and nothing else does:
  1. Keys compiled into the slopvac binary. The compiled plugins key is the only key for the first-party namespace `slopvac`, on every registry. Without a usable compiled plugins key, that namespace fails closed.
  2. Keys that the user pinned in the trust store for one pair of registry host and namespace, with `slopvac plugins pin`, which prints the fingerprint. The first pin binds the pair to the key. A key pinned for one pair signs only that pair. `plugins pin` refuses the namespaces `slopvac` and `core`.
- The binary compiles in two keys, and each has one job. The **release key** (`keys/slopvac-release.pub`) signs release archives and the revocation list, and it is the revocation root. It signs no plugin. The **plugins key** signs first-party plugins and the first-party plugin index. It signs no binary and no revocation list. The release key can revoke the plugins key (section 5.3). This split replaces a single first-party key that signed everything.
- The lockfile supplies no keys. It records the key id and `notBefore` of the signature that `sync` verified, so a later change shows up as `SignatureDiffersFromLock` (section 5.1).
- The index may display a key and its fingerprint. A first install shows the fingerprint and exits 2 until the user runs `slopvac plugins pin <namespace> <fingerprint>`, for data plugins and code plugins alike. No flag, `--yes` or environment variable pins a key on its own. `SLOPVAC_TRUST` names content hashes only and cannot pin a key. A fingerprint or a name in it is an error.
- A pinned key that differs from the index is a hard failure with exit 2. A pin stays in force whatever the index lists.
- A namespace binds to a key at first pin, per registry host. A plugin that claims the namespace `slopvac` must verify against the compiled plugins key. The id and namespace `core` belong to the built-in plugin, which is compiled in and never published. Any claim of `core` fails closed: a statement, lock entry, index entry, config entry or pin that names it is refused with exit 2.
- A key id inside a statement selects among the keys the policy already holds. It adds none.

### 5.3 Rotation and revocation

- **Rotation.** A publisher announces a new key with a rotation statement signed by the old key and by the new key. The statement names the namespace, both key ids, the whole new public key and a `notBefore` time. The host accepts a rotation only for a pinned key and shows the new fingerprint. The user confirms with `slopvac plugins rotate`. The rotation replaces the pin, so the old key signs nothing after it. Lock entries signed by the old key then fail `sync` until the publisher signs new releases and the user runs `update`. The first-party compiled set changes only with a slopvac release.
- **Compromised key.** The publisher cannot sign its own revocation if the key is lost, so a revocation is a statement signed by the release key, the revocation root. The list verifies against the release key only, and the plugins key never signs one. It lists key ids and artifact manifest digests. A listed key id may be that of the compiled plugins key. Then no first-party plugin and no first-party index verifies, and both fail closed until a slopvac release ships a new plugins key. The list cannot revoke the release key itself. Replacing the release key takes a new slopvac release (R8). The host caches the signed list, fetches it during `sync`, `update` and `outdated`, and checks it at every plugin spawn from the cache.
- **Effect.** A revoked key or digest fails `sync` with exit 2 and refuses to spawn. Trust records for a revoked digest are inert and `plugins status` flags them.
- **Freshness.** Each list carries a signed `expires` at most 30 days after its `issued` time, so the release key signs a new list at least every 30 days. An online command fails with exit 2 and keeps its cache when it cannot fetch the list, or when the served list has expired, because an expired list is a replayed old one. To run on the cache, the user must go offline explicitly.
- **Offline.** An offline run uses the cached list. A cached list past its `expires` prints a notice and does not block. A machine that never fetched a list runs with a notice too. The user chose this window (`slopvac-8l4.9.15`). The window is a residual risk (R3).
- **Rollback.** A statement carries `version` and `notBefore`, and the lockfile holds both as a floor. The verifier refuses an earlier version and, independently, an earlier `notBefore`, unless the caller passes the explicit downgrade flag. A new statement with the same version and an older `notBefore` is refused. A malformed floor time fails closed. The lockfile also fixes the exact `keyId` and `notBefore`, so `sync` rejects any other valid signature for the same digest. The implemented test is `tm25_an_older_version_or_older_not_before_is_refused_without_the_downgrade_flag` in `crates/slopvac-plugins/tests/sign.rs`.

Formats (implemented in `crates/slopvac-plugins/src/sign.rs` and `crates/slopvac-plugins/src/revoke.rs`):

- Both documents are canonical JSON as in section 5.1, parsed strictly, and signed in the prehashed minisign mode. A `type` field keeps one signed document from passing as another.
- A rotation statement holds `schema_version` (1), `type` (`rotation`), `namespace`, `oldKeyId`, `newKeyId`, `newKey` and `notBefore`, at most 4 KiB. `newKey` is the base64 line of the new `minisign.pub`. A minisign key id is unsigned metadata that any key can claim, so the host compares the offered new key with `newKey` byte for byte. The old key and the new key each sign it, in two `.minisig` texts. A key pin records the `notBefore` of the rotation that set it. A later rotation must carry a later `notBefore`. A rotation from or to a revoked key is refused, because a stolen key can sign a rotation to the thief's key.
- The revocation list holds `schema_version` (1), `type` (`revocations`), `issued`, `expires`, `keys` (key ids, which may include the compiled plugins key) and `digests` (manifest digests), at most 1 MiB. Each array is sorted and has no repeats. A fetched list with an earlier `issued` than the cached one is a rollback and fails the command. A fetched list with the same `issued` but different content is an equivocation, which fails the command and keeps the cache. The identical list refreshes normally.
- The list is an OCI artifact of type `application/vnd.slopvac.revocations.v1` at `ghcr.io/slopvac/slopvac-revocations:latest`. It has exactly two layers, the list (`application/vnd.slopvac.revocations.v1+json`) and its `.minisig` (`application/vnd.slopvac.signature.minisig.v1`). A signed empty list must be published there before the CLI ships.
- The cache is one private file in the per-user state directory. It holds the signed list, its signature and the unsigned fetch time. Every read verifies the signature again, so an edit to the signed list fails closed with exit 2 until an online run replaces it. An edit to the fetch time goes undetected. The time is only displayed, and freshness depends on the signed `expires` alone. A refresh re-reads, compares and replaces the cache under a per-user lock, so two refreshes cannot commit a newer list and then an older one.
- The spawn gate reads and verifies the cache at every spawn. A list cached after the gate was built still stops the next spawn, and a cache that fails verification refuses every spawn.

### 5.4 Trust scope

Trust records one **trust hash**. It must cover everything that can execute or change what executes:

- the plugin manifest, including the `PluginDeclaration` (hooks, network, features) and the argument list;
- every file of the extracted plugin tree, by relative path and content;
- for a Python plugin, the interpreter identity (version and build) and the locked, hash-pinned dependency set plus the file listing of the managed environment (section 6.6). The interpreter slot holds the version, the build and the path, size and sha256 of each interpreter executable. The environment slot holds a digest of the lock, the interpreter request and every entry of the environment: path, type, size, executable bit, sha256 and link target. Modification times are not in the hash;
- the manifest digest and the publisher key id.

Any change to any of these changes the trust hash and revokes trust (plugin-protocol.md section 4.3). `plugins update` shows the old and new declaration side by side. A data plugin has no code, needs no trust, and `plugins trust` refuses it. It still has a digest in the lockfile, because its rules decide the lint verdict (T1, A7).

Trust is stored per user, outside the project. A trust file inside a project is never read. Trust records the hash, the plugin id, the source, the key fingerprint, the declaration summary and the time. `SLOPVAC_TRUST` names hashes only, never a name or a tag.

### 5.5 Limits set by this model

The plugin protocol hands the numbers to this model. The per-plugin run caps are implemented as the `RunCaps` defaults in `crates/slopvac-plugins/src/host.rs`. The whole-run budget is `RunBudget` in `crates/slopvac-plugins/src/budget.rs` (`slopvac-8l4.9.33`).

The process limits (CPU time, open files, file size, core files and memory) are approved defaults. They are the `cpu_time`, `open_files`, `file_size` and `memory` fields of `RunCaps`, which `HostConfig::caps` carries. These values are fixed in 3.0: only code that builds a `HostConfig` sets them, and the configuration file has no key for them. The host sets them on Unix through the plugin launcher (`slopvac-8l4.9.13`). Windows has no process limits in 3.0 (`slopvac-8l4.18.8`).

| Limit | Value | Bead |
|---|---|---|
| Unsolicited messages and responses with no matching request from a plugin per run | 1,000, then `plugin-protocol-violation` | `slopvac-8l4.9.6` |
| Channel between reader thread and workers | bounded to twice `maxInFlight` frames; the reader blocks | `slopvac-8l4.9.6` |
| Bytes read from a plugin's standard output per run | 512 MiB, then kill | `slopvac-8l4.9.6` |
| Bytes read from a plugin's standard error per run | 128 MiB read and discarded, then kill; last 64 KiB kept | `slopvac-8l4.9.6` |
| Wall-clock time per plugin per run | 30 minutes, then kill and exit 2 | `slopvac-8l4.9.6` |
| Whole-run budget across all plugins | 2 GiB of standard output, 512 MiB of standard error and 4,000 unsolicited messages in total, then kill the offending plugin and exit 2 | `slopvac-8l4.9.33` |
| Message rate per second | No limit. Bounded channels give backpressure, and the byte and message caps bound the total work. | `slopvac-8l4.9.6` |
| CPU time per plugin process | 30 minutes, with `SIGXCPU` at the limit and `SIGKILL` 1 s later | `slopvac-8l4.9.13` |
| Open files, file size, core files | 1,024 files, 256 MiB, no core files | `slopvac-8l4.9.13` |
| Memory per plugin process | 4 GiB (`RLIMIT_DATA`) on Linux only | `slopvac-8l4.9.13` |
| Process count | No limit. `RLIMIT_NPROC` counts every process of the user, so the group kill and the wall-clock cap end a fork loop (R2). | `slopvac-8l4.9.13` |
| Fixer edits per document | 1,000 | `slopvac-8l4.9.6` |
| Fixer inserted bytes per document | the lesser of 1 MiB and four times the selected findings' span bytes plus 4 KiB | `slopvac-8l4.9.6` |
| OCI manifest or index body | 4 MiB | `slopvac-8l4.9.2` |
| OCI blob | exactly the descriptor size, at most 256 MiB per blob | `slopvac-8l4.9.2` |
| Redirects per request | 5, HTTPS only (loopback may use HTTP in tests) | `slopvac-8l4.9.2` |
| Credential helper output and time | 64 KiB and 10 s | `slopvac-8l4.9.2` |
| Compressed layer | 64 MiB | `slopvac-8l4.9.3` |
| Expanded size of one artifact | 256 MiB, with a compression ratio of at most 100 to 1 | `slopvac-8l4.9.3` |
| Entries in one archive | 10,000 | `slopvac-8l4.9.3` |
| Path length and depth in an archive | 1,024 bytes and 32 segments | `slopvac-8l4.9.3` |
| Signed statement | 64 KiB | `slopvac-8l4.9.5` |
| Entries, depth and file bytes in one managed Python environment | 20,000 entries, 32 levels and 1 GiB | `slopvac-8l4.9.31` |
| One `uv` run or interpreter probe during an environment build | 10 minutes, then kill | `slopvac-8l4.9.31` |
| Plugin index document | 32 MiB | `slopvac-8l4.9.8` |
| Regex pattern length in a data plugin | 4,096 bytes | `slopvac-8l4.3.20` |
| PCRE2 match, depth and heap limits | fixed in `slopvac-8l4.3.20`; a limit hit is an error, never a no-match | `slopvac-8l4.3.20` |

## 6. Threat register

Columns: L and I are likelihood and impact (L low, M medium, H high). "Owner" is the bead that builds the control. "Test" is the test that proves the control. H-numbers are tests of plugin-protocol.md section 12.1. TM-numbers are tests this model adds, and section 7 lists them again by acceptance heading.

### 6.1 Plugin host and subprocess

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| P1 | An oversized or malformed frame exhausts memory or desynchronises the stream. | D | CWE-770, CWE-190 | M/H | Check `Content-Length` with checked arithmetic before allocating. Stop at a framing error and kill. 1 MiB limit before `initialize`. | `slopvac-8l4.9.6` | H1, H2, H3, H25, TM-1 (oversized frames) |
| P2 | Nested JSON or duplicate keys crash the parser or split parsers. | D, T | CWE-674, CWE-1286 | M/M | Depth limit 64, reject duplicate keys and non-integer numbers. | `slopvac-8l4.9.6` | H25, H35 |
| P3 | A flood of small messages or responses burns CPU or fills a queue. | D | CWE-400, CWE-770 | M/M | Bounded channels with backpressure. Cap on unsolicited messages and per-run byte caps from section 5.5. | `slopvac-8l4.9.6`, `slopvac-8l4.9.33` | TM-2 (message flood: kill at the cap, memory stays flat) |
| P4 | A standard error flood fills the pipe and stalls the plugin, or consumes memory. | D | CWE-400, CWE-779 | H/M | A dedicated drain thread reads from spawn. A fixed ring keeps 64 KiB with no line buffering. Total read is capped, then the plugin dies. | `slopvac-8l4.9.6` | H16, TM-3 (stderr floods: 10 MiB and a single 1 GiB line without newline) |
| P5 | A plugin starts children that outlive the run or escape the process group. | D, E | CWE-404, CWE-459 | H/H | Kill the process group on Unix. Job objects on Windows: out of scope for 3.0 (no native Windows build); tracked in slopvac-8l4.18.8. On Linux, mark the host a child subreaper and kill reparented children. On macOS, walk the process tree before the kill and kill every descendant found. Drop guard and signal handler. | `slopvac-8l4.9.6` | H17, H18, TM-4 (descendant processes: one in the group, one that calls `setsid`, one that double-forks) |
| P6 | A plugin uses unbounded CPU, memory, files, disk or processes. | D | CWE-400, CWE-770 | M/M | Wall-clock cap per run. `setrlimit` for CPU time, open files, core size and file size on Unix. Job object limits on Windows: out of scope for 3.0 (no native Windows build); tracked in slopvac-8l4.18.8. Memory and process limits are best effort (R2). | `slopvac-8l4.9.13` | TM-5 (limit test: busy loop, allocation loop and fork loop end with a diagnostic and exit 2) |
| P7 | A plugin read or write blocks the host. | D | CWE-833, CWE-400 | M/M | The host writes on its own thread. Every deadline is wall-clock and ends in a kill. | `slopvac-8l4.9.6` | H4, H13, H32, H33 |
| P8 | A plugin lies about its capabilities: reports `network: false` and connects, or offers more hooks than declared. | S, E | CWE-693, CWE-807 | M/H | Spawn decisions come from the hashed manifest. A wider reply is `plugin-declaration-mismatch`. A false `network: false` is not detectable in 3.0 (R1). | `slopvac-8l4.9.6` | H19, H26 |
| P9 | A plugin returns hostile results: false spans, edits outside the finding, wrong limits. | T | CWE-20, CWE-1284 | H/M | Validate every result before use. Reject the whole document on an edit outside its envelope. Never clamp. | `slopvac-8l4.9.6` | H11, H27, H28, H29, H30 |
| P10 | An untrusted or modified plugin runs. | S, E | CWE-345, CWE-829 | M/H | Trust check before spawn on the trust hash. The host hashes the files, copies or opens the verified tree, and runs from a read-only, content-addressed directory. | `slopvac-8l4.9.4`, `slopvac-8l4.9.6` | H20, TM-6 (modify one byte after trust: no spawn) |
| P11 | Swap between hash check and exec (time of check to time of use). | T | CWE-367 | L/H | Run from the verified tree, mode 0555 files in a 0700 directory the user owns. Open the entry by descriptor where the platform allows. A swap by the same user is a residual risk (R6). | `slopvac-8l4.9.6` | TM-7 (swap the entry file between check and exec in a hook-point test) |
| P12 | The environment leaks secrets or lets the plugin change loader behavior. | I, E | CWE-526, CWE-426 | M/H | Allowlist: host-set `PATH`, a locale, variables the manifest names and the user allowed. No `HOME`, tokens, CI variables, `PYTHON*`, `LD_*`, `DYLD_*`. All descriptors except 0 to 2 are closed. | `slopvac-8l4.9.6` | TM-8 (a plugin prints its environment: only the allowlist appears) |
| P13 | A trusted plugin turns hostile after an update. | E | CWE-1357 | M/H | A new version is a new hash. Updates display declaration changes; the spawn gate requires explicit trust for each new hash. Revocation list (K5 and K6 in section 6.3). | `slopvac-8l4.9.4` | TM-9 (update to a new hash: spawn refused, diff printed) |
| P14 | The plugin's standard error leaks document text into host logs. | I | CWE-532 | M/L | The host cannot redact free text. Stderr is shown only after a failure (last 4 KiB) or under `SLOPVAC_LOG`. Plugin docs say not to log content. | `slopvac-8l4.9.6`, `slopvac-8l4.12.1` | TM-10 (stderr text absent from a clean run's output) |
| P15 | Unicode control sequences in plugin text reach a terminal. | T, S | CWE-150 | H/M | Strip them with the shared sanitiser of O1. | `slopvac-8l4.9.32` | TM-11 |

### 6.2 Plugin output, fixers, recipes and providers

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| O1 | Plugin text carries terminal escapes, OSC 8 links, C1 controls or bidi overrides into the terminal, an agent's context, or CI logs. | T, S | CWE-150, CWE-451 | H/M | One sanitiser for all plugin-origin text: strip C0 (except newline and tab), C1, DEL, escape sequences and bidi override characters before any terminal or log write. Applies to reporter output, messages, stderr tails and error text. | `slopvac-8l4.9.32`, `slopvac-8l4.12.2` | TM-11 |
| O2 | Plugin text injects Markdown or HTML into reports, or script into an HTML report. | T, E | CWE-79, CWE-116 | M/M | Escape for the target format on output. The HTML renderer escapes all plugin text and emits no raw HTML, no `javascript:` or `data:` links and no remote resources. | `slopvac-8l4.9.32`, `slopvac-8l4.12.2` | TM-12 |
| O3 | SARIF or CodeClimate fields carry hostile URIs or paths. | T | CWE-74, CWE-22 | M/M | Locations are relative project paths. `helpUri` and links allow `https` only. JSON encoding of every string. | `slopvac-8l4.9.32`, `slopvac-8l4.12.2` | TM-12 |
| O4 | Newlines in a message become GitHub Actions workflow commands (`::set-output`, `::error`) or forge annotations. | T, E | CWE-93, CWE-117 | M/H | The Action escapes `%`, `\r`, `\n` and `::` for every annotation. The text reporter writes a line that starts with `::` only after it escapes the start. | `slopvac-8l4.9.32`, `slopvac-8l4.13.6` | TM-13 |
| O5 | A fixer rewrites more than the selected findings through many edits. | T | CWE-20 | M/M | Edit-in-envelope rule, plus the per-document limits in section 5.5. Over a limit, the document fails with `plugin-output-invalid`. | `slopvac-8l4.9.6`, `slopvac-8l4.3.17` | H27, H28, H29, TM-14 (1,001 edits rejected; inserted bytes over the cap rejected) |
| O6 | Applying edits or recipes tears a file, loses an update, or follows a swapped path. | T, D | CWE-367, CWE-362 | M/M | Write a temporary file in the same directory, check the source is unchanged (size, mtime and content hash), check the staged file is still the one created (device and inode) before the rename, rename atomically. Open with no-follow flags and parent descriptors. Refuse a hardlinked target (link count above 1) and a path whose parent chain contains a symlink that leaves the root. | `slopvac-8l4.9.14` | TM-15 (rename race, hardlink and symlink swap tests); `a_staged_file_replaced_before_the_rename_fails_closed` (a replaced temporary file fails the write) |
| O7 | A recipe targets a sensitive in-project file: `.git/`, hooks, `.slopvac/` state, CI workflows, `.env*`, `.gitmodules`, `.gitattributes`, `slopvac.toml`, `.mise.toml`, `.envrc`, `.vscode/tasks.json`. | E | CWE-73, CWE-22 | M/H | A deny list the host enforces. A recipe may touch a denied path only after a separate approval per path, in a command that allows writes (`--allow-sensitive <path>`). The plan output marks them. No plugin sets the flag. | `slopvac-8l4.9.14` | TM-16 (each denied path refused without the flag, nothing written) |
| O8 | A recipe writes agent harness files (`AGENTS.md`, hooks, MCP server entries, `.claude/`, `.opencode/`) that later run code with an agent's rights. Writing these files is the recipe's purpose, and it is also an escalation path. | E | CWE-94, CWE-829 | M/H | Show the full plan and content before applying. Hook-like and command-bearing entries are marked in the plan. Apply only with an explicit write command, only from a command that allows writes. | `slopvac-8l4.9.14` | TM-17 (a plan with a hook entry prints a marked warning and writes nothing under `--dry-run`) |
| O9 | A recipe path escapes the root: absolute, `..`, Windows drive, case-folding tricks, reserved device names, or a symlink. | T, E | CWE-22, CWE-59, CWE-178 | M/H | Host check is the boundary (plugin-protocol.md section 8.2). Resolve symlinks, compare case-insensitively on case-insensitive file systems, refuse reserved device names (CON, NUL, COM and LPT, with superscript digits), trailing dots or spaces, and a colon (NTFS streams) on every platform. This also covers WSL projects on `/mnt`. | `slopvac-8l4.9.14` | H22, TM-18 (Windows path forms and case-fold collisions) |
| O10 | A provider receives more text than the review type needs, or secrets from the document. | I | CWE-201, CWE-359 | M/H | `review --dry-run` prints the payload. Only the review type's text is sent (policies.md section 1.5). A provider runs only when trusted, named in config and online. The plugin may still forward anything it received (R4). | `slopvac-8l4.9.6`, `slopvac-8l4.11.3` | N12, TM-19 (payload equals the dry-run output) |
| O11 | A provider's credentials reach the wrong plugin or the logs. | I | CWE-522, CWE-532 | L/H | A provider key goes in only through an explicit config entry for that plugin, and only to that plugin's environment. Never to other plugins. Logs strip tokens (policies.md section 1.7). | `slopvac-8l4.4.2`, `slopvac-8l4.9.6` | R4, TM-20 (a second plugin's environment holds no provider key) |
| O12 | Prompt injection: a document tells the provider what to output, and the output reaches a fixer, a recipe or a verdict. | T, E | CWE-1427 | H/M | Provider output is marked untrusted in every report. The host has no code path from provider output to a fixer or a recipe in 3.0. It cannot change a lint verdict. | `slopvac-8l4.11.3`, `slopvac-8l4.9.6` | TM-21 (a provider returning edit-like text cannot reach `fixer/fix` or `recipe/plan`) |
| O13 | Plugin declaration drifts across updates: a new version adds `network: true` or a hook. | E | CWE-693 | M/M | The declaration is in the trust hash. `plugins update` prints a declaration diff before the trust command is available. | `slopvac-8l4.9.4` | TM-9 |

### 6.3 Trust store, signatures and keys

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| K1 | An attacker substitutes a key in the auto-rebuilt index so a hostile artifact verifies. | S, T | CWE-345, CWE-347 | M/H | The index is not an authority (section 5.2). A first install pins a key only through an explicit command that shows the fingerprint. A pinned key never changes from the index. | `slopvac-8l4.9.5`, `slopvac-8l4.9.8` | TM-22 (substituted index key: sync exits 2, nothing extracted) |
| K2 | A layer or the manifest is swapped while the signature still verifies (signature covers the wrong bytes). | T | CWE-347, CWE-345 | M/H | The statement lists every other layer digest and the config digest. Any extra, missing or reordered layer, or a mismatched annotation, invalidates the artifact (section 5.1). | `slopvac-8l4.9.5` | TM-23 (add a layer, drop a layer, edit an annotation, swap the config: each fails) |
| K3 | A plugin claims a first-party, reserved or foreign namespace to trade on its name. | S | CWE-290 | M/M | The namespace binds to a key per registry host. `slopvac` verifies against the compiled plugins key only and is never pinnable. `core` fails closed everywhere. | `slopvac-8l4.9.1`, `slopvac-8l4.9.5` | TM-24 |
| K4 | A signed but old artifact replays (rollback) to bring back a flaw. | T | CWE-494 | M/M | The statement holds `version` and `notBefore`. `verify` refuses an earlier version and, separately, an earlier `notBefore` than the lockfile floor, and `sync` requires the lock's exact `keyId` and `notBefore`. The lockfile pins the manifest digest. | `slopvac-8l4.9.3`, `slopvac-8l4.9.5` | TM-25 |
| K5 | A compromised publisher key signs a hostile update. | S, E | CWE-321 | L/H | Revocation list signed by the release key (section 5.3). Rotation needs an old-key statement. Trust still needs the user's command for each new hash. | `slopvac-8l4.9.15` | TM-26 (a revoked key or digest fails sync and refuses to spawn) |
| K6 | A revocation list is withheld, stale or forged. | T, D | CWE-299 | M/M | The list is signed by the release key and cached with its time. Online commands fetch it. A stale offline cache warns. A forged list fails verification. | `slopvac-8l4.9.15` | TM-27 (forged and stale lists) |
| K7 | The trust store is tampered with or is read from the project. | T, E | CWE-732, CWE-807 | M/H | The store lives in the per-user state directory, mode 0600, written atomically under a lock. A project file named like the store is ignored. | `slopvac-8l4.9.4` | TM-28 (a trust file in the project grants nothing) |
| K8 | A tool, agent or flag records trust. | E | CWE-358 | M/H | Only `plugins trust` and `SLOPVAC_TRUST` write trust. `sync`, `update` and `--yes` never do. No prompt exists. | `slopvac-8l4.9.4` | N8 |
| K9 | A hostile pull request edits CI configuration to set `SLOPVAC_TRUST` for its own plugin. | E | CWE-829 | M/H | `SLOPVAC_TRUST` names hashes. Docs tell projects to protect workflow files with code owners. Slopvac cannot enforce review (R5). | `slopvac-8l4.9.4`, `slopvac-8l4.13.6` | TM-29 (the variable names a hash that differs from the plugin on disk: no spawn) |
| K10 | A signature on a data plugin is skipped because it has no code. | E | CWE-345 | M/M | Data plugins are signed and verified like code plugins. Their rules decide verdicts and run regexes. | `slopvac-8l4.9.5`, `slopvac-8l4.9.1` | TM-30 |
| K11 | Local folders and git commits carry no signature. | S | CWE-494 | M/M | A git pin names a full commit hash, and the trust hash covers the content. A local folder is hashed at each run. A moving branch name is refused. | `slopvac-8l4.9.3`, `slopvac-8l4.9.4` | TM-31 |
| K12 | Signing releases or `self update` lets an attacker substitute the binary. | T, E | CWE-494 | L/H | Verify the minisign signature on the archive before replacement. Keys compiled into the binary. | `slopvac-8l4.13.3`, `slopvac-8l4.12.4` | TM-32 (a tampered archive is refused) |
| K13 | The plugins key leaks, and the thief signs hostile first-party plugins or a hostile first-party index. | S, T | CWE-321 | L/H | The plugins key is separate from the release key, so it cannot sign a binary or a revocation list. The release key revokes it through the revocation list, and the `slopvac` namespace then fails closed until a release ships a new plugins key. Code plugins still need `plugins trust` for each new hash. | Release process | TM-60 |

### 6.4 OCI client, credentials and network sources

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| N1 | A blob differs from its digest, or a registry lies about size. | T | CWE-345, CWE-1284 | M/H | Stream to a temporary file with a running SHA-256 and a size check against the descriptor. Verify before decompressing and before signature use. | `slopvac-8l4.9.2` | TM-33 (wrong digest, short body, long body) |
| N2 | A manifest or index exceeds the limit or is shaped to cause confusion (unknown media types, nested indexes). | D, T | CWE-770, CWE-20 | M/M | 4 MiB manifest limit. Accept the known media types. No nested index resolution beyond one level. | `slopvac-8l4.9.2` | TM-34 |
| N3 | An auth challenge sends credentials to an attacker host. A token `realm` points elsewhere. | I | CWE-522, CWE-918 | M/H | The realm must be HTTPS and on the registry's host or a listed token host of the same registry. Credentials go only to the registry named in the reference. | `slopvac-8l4.9.2` | TM-35 (hostile realm is refused; no credentials sent) |
| N4 | A redirect leaks the token to a CDN host or downgrades to HTTP. | I | CWE-601, CWE-319 | M/H | Drop `Authorization` on a cross-host redirect (measured in `slopvac-8l4.2.17`). HTTPS only. Redirect cap of 5. | `slopvac-8l4.9.2` | TM-36 |
| N5 | TLS validation is weakened or the system trust store is bypassed. | S | CWE-295 | L/H | rustls with the platform verifier. No flag disables verification. A private CA comes from the system store. | `slopvac-8l4.9.2` | TM-37 (no option disables verification) |
| N6 | A credential helper named in the Docker config, or planted on `PATH`, runs an unintended program or injects arguments. | E | CWE-78, CWE-88, CWE-426, CWE-427 | L/H | The helper name must match `[a-z0-9_.-]+`, run as `docker-credential-<name>` without a shell, with a 10 s limit and a 64 KiB output cap. The registry host is validated before it goes to helper input. The config path is the user's `DOCKER_CONFIG` or home directory only, never the project. Static logins (environment, `auths`, registry auth files) are tried before any helper starts. The helper is found on `PATH` through trusted directories only: absolute, outside the project, link-resolved, and owned by root or the user. Others cannot write them. A directory that a group can write is refused unless the group is `admin`, looked up by name, and the process belongs to it. No other group counts, because directory services hide members from `/etc/group`. The resolved path is the one that runs. The caller must name the project; without it no helper starts. A refused helper is not started. The request goes on without a login. If the registry then demands one, the operation fails with the code `credential-helper-untrusted` and a redacted directory, and the message points to a static `auths` login or the environment variables. | `slopvac-8l4.9.2` | TM-38 (names with `/`, `..`, `;` or spaces are refused; a hanging helper times out; a helper in a relative, empty, project-local, world-writable or shared-group directory is not run; a helper behind a root-owned `admin` directory is run) |
| N7 | Credentials appear in logs, errors, process arguments or the lockfile. | I | CWE-532, CWE-214 | M/H | Credentials travel only in memory and headers. URL user-info is stripped before logging. | `slopvac-8l4.9.2`, `slopvac-8l4.12.1` | R4 |
| N8 | A plugin obtains the user's registry credentials. | I | CWE-522 | L/H | The plugin environment is an allowlist (P12). The host omits tokens and the docker config path from the plugin environment. | `slopvac-8l4.9.6` | TM-8 |
| N9 | A release archive is fetched over HTTP, or its digest is not checked. | T | CWE-494, CWE-319 | M/H | HTTPS only. The digest comes from the lockfile and is checked before extraction. A first fetch without a digest fails `sync` unless the user adds the digest. | `slopvac-8l4.9.3` | TM-39 |
| N10 | A git pin executes code on clone: hooks, `core.fsmonitor`, submodules, `ext::` or `file::` transports, filters, LFS. | E | CWE-94, CWE-78 | M/H | Clone in a scratch directory with `protocol.allow=never` except `https` (and `ssh` by explicit config), `GIT_CONFIG_NOSYSTEM`, no templates, `core.hooksPath=/dev/null`, no submodule update, no filters. Check out only the pinned full hash, then hash the content. | `slopvac-8l4.9.3` | TM-40 (a repository with hooks, a submodule and a filter runs nothing) |
| N11 | The index serves oversized or hostile text, or tampered key data. | T, D | CWE-770, CWE-150 | M/M | 32 MiB limit. Descriptions pass the sanitiser (O1) before display. Key data is display only (section 5.2). | `slopvac-8l4.9.8` | TM-41 |
| N12 | Typosquatting and namespace confusion. | S | CWE-1357 | M/M | Namespaced registries. `plugins trust` shows plugin id, source and key fingerprint. Search results show the publisher key fingerprint. | `slopvac-8l4.9.1`, `slopvac-8l4.9.8` | TM-24 |
| N13 | Publishing leaks the token, or pushes to the wrong repository. | I | CWE-522 | L/M | `plugins publish` shows the destination and requires `write:packages` credentials from the environment or the helper. | `slopvac-8l4.9.9` | TM-42 |

### 6.5 Archive extraction and the cache

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| E1 | A path escapes the target: `..`, absolute, drive letters, backslashes, NUL, overlong. | T, E | CWE-22 | H/H | Allow only relative paths of normal segments, within the length and depth limits. Create nothing outside the staging directory. | `slopvac-8l4.9.3` | TM-43 (malicious archive corpus: traversal, absolute, drive, NUL) |
| E2 | Symlinks, hardlinks, devices, FIFOs or sockets in the archive. | T, E | CWE-59, CWE-61, CWE-62 | H/H | Accept regular files and directories only. Reject every other entry type, including hardlinks. 3.0 plugins need no symlinks. | `slopvac-8l4.9.3` | TM-43 |
| E3 | A link or entry written earlier redirects a later entry. | T | CWE-59 | M/H | Created entries are never links (E2). Extraction uses no-follow opens and refuses to write over an existing entry. A duplicate path fails the artifact. | `slopvac-8l4.9.3` | TM-43 |
| E4 | Decompression bomb. | D | CWE-409, CWE-400 | H/M | Streaming decompression with a running count. Stop at 256 MiB expanded, a ratio of 100 to 1, or 10,000 entries. | `slopvac-8l4.9.3` | TM-44 (a bomb is stopped with bounded disk and memory) |
| E5 | Entry modes grant setuid, setgid or world write, or odd ownership. | E | CWE-732 | M/M | Ignore archive modes and owners. Files become 0644 or 0755 as the manifest declares an entry point. | `slopvac-8l4.9.3` | TM-43 |
| E6 | Case-insensitive or Unicode-normalising file systems merge two entries. | T | CWE-178, CWE-176 | M/M | Reject an archive whose paths collide after case folding and NFC normalisation, and reserved device names (CON, NUL, COM and LPT, with superscript digits), trailing dots or spaces, and colons, on every platform. | `slopvac-8l4.9.3` | TM-43 |
| E7 | Extraction before verification, or a half-extracted tree is used. | T | CWE-345, CWE-459 | M/H | Order: digest, signature, then extract into a staging directory beside the cache, hash, then rename atomically into the content-addressed path. A failure removes the staging directory. | `slopvac-8l4.9.3` | TM-45 (an interrupted extraction leaves no usable entry) |
| E8 | A tampered cache entry loads. | T, E | CWE-345, CWE-732 | M/H | Cache paths derive from the digest. Loading a data plugin recomputes its digest. Spawning a plugin recomputes the trust hash (P10). `SLOPVAC_CACHE_DIR` is treated as untrusted content. A mismatch evicts the entry and fails with exit 2. | `slopvac-8l4.9.3`, `slopvac-8l4.9.6` | TM-46 (tampered caches: edit a file, add a file, delete a file, replace a directory with a symlink) |
| E9 | The cache directory or lock file is world-writable, or its temporary files are predictable. | T | CWE-377, CWE-732 | L/M | Create the cache 0700. Use random temporary names with exclusive creation. Take a lock through `fs4`. | `slopvac-8l4.9.3` | TM-47 |
| E10 | A lockfile edit by a pull request points at attacker content. | T | CWE-345 | M/H | The lockfile pins digests and never a tag. A changed digest still has to verify against a pinned key, and code plugins still need the user's trust. In CI, `sync` fails on any mismatch and leaves the file as it found it (N7 in policies.md). | `slopvac-8l4.9.3` | N7, TM-48 (a lockfile with a digest signed by an unpinned key fails) |

### 6.6 Managed Python environments

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| Y1 | Installing dependencies runs code before any trust decision, through sdist builds, `setup.py`, build backends or install hooks. | E | CWE-94, CWE-829 | H/H | Build environments only from locked, hash-pinned wheels (`--only-binary :all:`, `--require-hashes`, `--no-deps`) with no build isolation steps that run project code. A plugin with a source-only dependency is rejected, or ships the dependency vendored inside the signed artifact. | `slopvac-8l4.9.31` | TM-49 (an sdist dependency is refused; no build backend runs) |
| Y2 | Dependency substitution or change after trust: index confusion, a moved release, a changed lock. | T, S | CWE-1357, CWE-829 | M/H | The lock with hashes ships in the signed artifact. The environment is built only from it. The lock and the resulting file listing are part of the trust hash (section 5.4). No resolver runs at install time. | `slopvac-8l4.9.31`, `slopvac-8l4.9.4` | TM-50 (dependency changes: edit one pin, add a package, swap a wheel: each changes the trust hash and refuses spawn) |
| Y3 | User or environment configuration redirects the installer: `PIP_INDEX_URL`, `UV_*`, `pip.conf`, `.netrc`, `PYTHONPATH`. | S, E | CWE-426, CWE-807 | M/H | Run the installer with a scrubbed environment and its configuration discovery turned off. Run Python in isolated mode (`-I`) so `PYTHON*` variables, the user site and the current directory do not load. | `slopvac-8l4.9.31` | TM-51 (hostile `PIP_INDEX_URL`, `PYTHONPATH` and `usercustomize` have no effect) |
| Y4 | A `.pth` file, `sitecustomize` or entry point runs code at interpreter start. | E | CWE-94 | M/H | The file listing in the trust hash covers every file in the environment, so a new `.pth` changes the hash. Bytecode is compiled at build time and `PYTHONDONTWRITEBYTECODE` is set. | `slopvac-8l4.9.31` | TM-50 |
| Y5 | The environment changes after trust: a running plugin edits its own site-packages. | T | CWE-732, CWE-367 | M/M | Make the environment read-only after the build. Verify at every spawn against the stored listing by paths, types, sizes, modification times, executable bits and link targets, without hashing. Hash every file in `plugins status`, `doctor` and `plugins trust`. The section after this table records the trade-off. Same-user writes remain a residual risk (R6). | `slopvac-8l4.9.31` | TM-52 (modify a file in the environment: spawn refused) |
| Y6 | The interpreter is substituted: a `python` found on `PATH` or a project-local one. | S, E | CWE-426, CWE-427 | M/H | Use an interpreter that the managed build recorded, by absolute path. Its identity is in the trust hash. | `slopvac-8l4.9.31` | TM-53 |
| Y7 | Environment disk use grows without bound. | D | CWE-400 | L/L | Environments are cache entries with the extraction limits of section 5.5, and `doctor` lists them for removal. | `slopvac-8l4.9.31` | TM-44 |

**Environment verification cost (Y5), decided 2026-11-15.** Slopvac checks an environment at these points:

- **At every spawn.** The spawn gate reads every directory of the environment and the metadata of each entry, and hashes nothing, so file sizes do not add to its cost. The check refuses an added, removed or retyped entry, such as a new `.pth` file. It also refuses a change in size, modification time, executable bit or link target, and a change in the interpreter's size or modification time.
- **In `plugins status`, `doctor` and `plugins trust`.** The full check hashes every file and every interpreter executable. Trust is never recorded for an environment that fails it. `plugins trust`, `plugins status` and `doctor` run it. Status calls `sync::status_with_python`, which reports each environment as verified, missing or failed.

The spawn check misses an edit that keeps the size and restores the modification time. That edit needs write access as the user, which is residual risk R6, and the next full check reports it. The environment is read-only after the build, so a plugin must change file modes before it can write there. The test `tm52_a_same_size_edit_with_the_old_mtime_is_caught_by_the_full_check` records this gap.

**How the build meets Y1 to Y4 and Y6.**

- A plugin ships a PEP 751 `pylock.toml` and its wheels inside the signed tree.
- Before any installer runs, slopvac refuses a package with an `sdist`, `vcs`, `directory` or `archive` source. It also refuses a wheel without a path in the tree and a wheel whose sha256 differs from the lock.
- Before any installer runs, slopvac also reads each wheel's ZIP central directory. It refuses a ZIP64 archive and a lock whose wheels declare more entries or bytes than the environment bounds of section 5.5.
- The same check refuses a wheel entry that would land where the build or the interpreter runs it before trust:
  - a script named `python*`, `activate*`, `deactivate*` or `pyvenv.cfg`;
  - data under `bin`, `Scripts`, `lib`, `lib64` or `include`, or a root `pyvenv.cfg`;
  - in `site-packages`, any `.pth` file and any top-level `sitecustomize` or `usercustomize` module;
  - an unknown install scheme, and a path that is absolute, has `.` or `..` segments or uses a backslash.
- After `uv venv`, the build records the interpreter it resolves to, that interpreter's sha256 and the sha256 of `pyvenv.cfg`. Before it compiles bytecode and before it probes the interpreter, it checks all three again and stops the build on any change.
- uv runs with an empty environment except for `UV_PYTHON_INSTALL_DIR`. The flags `--no-config` and `--no-project` turn off configuration discovery.
- `uv venv --managed-python` uses only a uv-managed interpreter, never one found on `PATH`. Offline mode adds `--offline --no-python-downloads`.
- `uv pip install --offline --no-index --no-build --no-deps --require-hashes` installs from the tree's wheel directories only. Each package is pinned to its exact version and locked hashes.
- No code from a wheel runs during the build. uv's `--compile-bytecode` starts the interpreter with `site` enabled, which runs any `.pth` file, so the build does not use it. The build compiles bytecode and probes the interpreter with `-I -S` instead.
- The plugin runs as `<env>/bin/python -I -B`, by absolute path. `TrustGate` returns an error for any other program or first arguments of a managed plugin.

**Residual gaps.**

- The bounds check reads the sizes that a wheel declares. uv expands the wheel, and slopvac checks the bounds of the result only after the install.
- Slopvac refuses to build, load or check a managed environment on Windows (`UnsupportedPlatform`), so a Python plugin with a managed environment does not run there. 3.0 ships no native Windows support; any later work is tracked in `slopvac-8l4.9.37`.
- The startup-hook refusal also refuses packages that ship a `.pth` file for their own use, such as `setuptools`.
- CI has no uv yet, so the `live_` tests skip there. Tests with a scripted fake uv cover the build controls without uv. The fake runs the interpreter with `site` enabled when it receives `--compile-bytecode`, as uv does. A request to `slopvac-8l4.14` asks for uv in the Rust CI job and `SLOPVAC_PYENV_LIVE=1`.
- The interpreter identity covers the interpreter executables. The standard library of the uv-managed installation sits outside the environment, and a change there needs write access as the user (R6).
- uv writes install times into the environment, so a rebuilt environment gets a new trust hash and the user must trust the plugin again.
- The build passes no proxy variables to uv, so an interpreter download through a proxy fails. The user can then install the interpreter with `uv python install` and build offline.

### 6.7 Data plugins and regular expressions

| ID | Threat | STRIDE | CWE | L/I | Mitigation | Owner | Test |
|---|---|---|---|---|---|---|---|
| X1 | A plugin ships a pattern with catastrophic backtracking that hangs or exhausts a run. | D | CWE-1333, CWE-400 | M/M | Set PCRE2 match, depth and heap limits and a JIT stack cap on every compile, at the values `slopvac-8l4.3.20` fixes. Matching has no global state that a plugin can raise. | `slopvac-8l4.3.20`, `slopvac-8l4.3.7` | TM-54 (a nested-quantifier pattern against a hostile document ends at the limit) |
| X2 | A limit hit is read as no match, so a plugin silently passes bad text or a plugin hides a finding. | T, R | CWE-755, CWE-703 | M/H | A limit error is a distinct, reported failure: the rule is incomplete, the run exits 2, and the diagnostic names the rule and document. Never "no match". | `slopvac-8l4.3.7` | TM-55 (limit hit gives exit 2 and a diagnostic) |
| X3 | An oversized pattern, alternations of thousands of branches or many rules slow compilation. | D | CWE-770 | M/M | Pattern length 4,096 bytes. Total rules and compiled size capped per plugin and per run. Compilation of an invalid or over-limit pattern fails the plugin. | `slopvac-8l4.3.20`, `slopvac-8l4.9.1` | TM-56 |
| X4 | Pattern options change engine behavior: verbs such as `(*LIMIT_MATCH=)`, callouts, `\K` abuse, recursion. | T, D | CWE-1333 | L/M | Compile with the plugin API's fixed flags. A pattern verb may only lower a limit. Callouts are not available (no callback is set). | `slopvac-8l4.3.20` | TM-57 |
| X5 | A 100 MiB document makes a safe pattern slow. | D | CWE-400 | M/M | Per-document byte cap and per-rule time accounting. The run reports the slow rule, exits 2 at the cap. | `slopvac-8l4.3.20` | TM-58 |
| X6 | A hostile plugin shapes verdicts: severity, suppression or ignore rules that hide real findings. | T | CWE-807 | M/H | Project config, not a plugin, decides which plugins load. Plugin-provided suppression stays within the plugin's own rules. `--explain-config` shows provenance. | `slopvac-8l4.9.1`, `slopvac-8l4.4.2` | TM-59 (a plugin cannot suppress another plugin's rule) |

## 7. Coverage

### 7.1 Acceptance list of `slopvac-8l4.9.11`

| Required tests | Threats | Tests |
|---|---|---|
| Malicious archives | E1 to E7 | TM-43, TM-44, TM-45 |
| Substituted index keys | K1, K2, K3, N11 | TM-22, TM-23, TM-24, TM-41 |
| Dependency changes | Y1, Y2, Y4, Y5, Y6, P13 | TM-9, TM-49, TM-50, TM-52, TM-53 |
| Oversized frames | P1, P2 | H1, H2, H3, H25, H35, TM-1 |
| Stderr floods | P4, P14 | H16, TM-3, TM-10 |
| Descendant processes | P5 | H17, H18, TM-4 |
| Tampered caches | E8, E9, E10, P10 | TM-6, TM-46, TM-47, TM-48 |

### 7.2 Hand-offs from plugin-protocol.md section 11.2

| Hand-off | Threats |
|---|---|
| Resource limits | P3, P4, P6 |
| Subprocess containment | P5, P6, P8, R1 |
| Message size and shape | P1, P2, P3 |
| Path exposure | O9 |
| Trust scope | P10, P11, Y2, Y4, Y5, section 5.4 |
| Output as input | O1 to O4 |
| Provider data flow | O10, O11, O12 |
| Malicious plugin | P8, P9 |
| Trust is not containment | P13, R1 |
| Channels and output volume | P3, P4, section 5.5 |
| Atomic application | O6 |
| Sensitive in-project targets | O7, O8 |
| Plugin text in output | O1 to O4 |
| Provider disclosure | O10 to O12 |
| Declaration drift | O13 |
| Fixer envelope | O5 |
| Duplicate keys and parser differences | P2 |
| Log leakage | P14, N7 |
| Revocation of trusted artifacts | K5, K6 |

### 7.3 Test definitions

The TM tests are proposed. They run in the Rust test suites of the owning bead, against fixtures built in the test code. A test that needs a hostile registry runs against a local `registry:2` or `zot` container or a scripted HTTP server on loopback. A test of the plugin host uses a scripted fake plugin, as in plugin-protocol.md section 12. Each test asserts both the exit code (2 for an incomplete run) and the absence of the side effect: no file outside the staging directory, no spawn, no credentials on the wire.

| ID | Setup | Expected |
|---|---|---|
| TM-1 | A plugin sends a header of `Content-Length: 4294967296` and a 16 MiB + 1 byte body. | Framing error before any allocation, kill, no spawn reuse. |
| TM-2 | A plugin sends 10,000 notifications and responses with unknown ids. | Kill at 1,000, bounded memory. |
| TM-3 | A plugin writes 10 MiB to standard error, and in another test a single 1 GiB line with no newline. | The host keeps reading, keeps 64 KiB, kills at the cap, and its memory stays flat. |
| TM-4 | A plugin starts three children: one in its group, one after `setsid`, one by double fork. It then hangs. | After the kill sequence, a process probe finds none of them on Linux. Windows: out of scope for 3.0 (no native Windows build); tracked in slopvac-8l4.18.8. On macOS, the tree-walk test finds none that existed at kill time. |
| TM-5 | A plugin loops on CPU, allocates until it fails, or forks repeatedly. | Ended by a limit or the wall-clock cap, with a diagnostic and exit 2. |
| TM-6 | Trust a plugin, then change one byte of an installed file. | `plugin-hash-mismatch`, no spawn. |
| TM-7 | A test hook swaps the entry file between the hash and the exec. | The swap does not execute, or the test records the residual (R6). |
| TM-8 | A plugin prints its environment. | Only the allowlist. No token, `HOME` or CI variable. |
| TM-9 | Install version 1, trust it, update to version 2 with a changed declaration. | Spawn refused, the diff printed, trust not recorded. |
| TM-10 | A plugin writes document text to standard error and succeeds. | The text appears in no output unless `SLOPVAC_LOG` is set. |
| TM-11 | Plugin messages and stderr contain C0, C1, OSC 8, CSI and bidi characters. | None reaches a terminal write. Newline and tab remain. |
| TM-12 | A reporter and message contain `<script>`, Markdown links and `javascript:` URIs. | Escaped in Markdown, HTML and SARIF. |
| TM-13 | A message contains `\n::error::x` and `%0A`. | The Action output holds no workflow command. |
| TM-14 | A fixer returns 1,001 edits, and edits whose inserted bytes pass the cap. | The document fails with `plugin-output-invalid`. Nothing is applied. |
| TM-15 | Race a rename, swap a target for a hardlink, swap a parent for a symlink during application. | Nothing outside the root changes. The file is whole or untouched. |
| TM-16 | A recipe targets each denied path. | Refused without the per-path flag. Nothing is written. |
| TM-17 | A recipe plan contains a hook and an MCP entry. | The plan marks them. A dry run writes nothing. |
| TM-18 | Recipe paths in Windows forms, with case-fold collisions and reserved names. | Refused. |
| TM-19 | A review with a provider. | The bytes sent equal the `--dry-run` payload. |
| TM-20 | Two trusted plugins, one a provider with a key. | The second plugin's environment holds no key. |
| TM-21 | A provider returns text that looks like edits or a plan. | It cannot reach `fixer/fix`, `recipe/plan` or the verdict. Reports mark it untrusted. |
| TM-22 | The index lists a new key for a pinned plugin, and for a new plugin. | Exit 2. The new plugin waits for the explicit pin. Nothing extracted. |
| TM-23 | Valid signature; then add a layer, remove a layer, reorder layers, change an annotation, swap the config. | Each fails verification. |
| TM-24 | A plugin claims the namespace `slopvac` under a key other than the compiled plugins key, and a pin for `slopvac` or `core` is attempted. | Rejected, exit 2. |
| TM-25 | Offer an earlier version, and separately the same version with an older `notBefore`. | Each is a `Rollback` error without the downgrade flag. |
| TM-26 | A revocation list names a key and a digest. | `sync` exits 2. The plugin does not spawn. `status` flags the trust record. |
| TM-27 | A forged list, and a stale cache offline. | The forged list is rejected. The stale cache prints a notice and runs. |
| TM-28 | Put a trust file in the project. | It grants nothing. |
| TM-29 | `SLOPVAC_TRUST` names the wrong hash. | No spawn. |
| TM-30 | A data plugin with no signature, and with a bad one. | `sync` exits 2. |
| TM-31 | Pin a branch name, and a full hash whose content changed. | The branch is refused. The hash mismatch is reported. |
| TM-32 | A release archive with a bad signature. | `self update` leaves the binary alone. |
| TM-33 | A blob with the wrong digest, a short body and a long body. | Each fails. No usable cache entry. |
| TM-34 | A 5 MiB manifest and an unknown media type. | Refused. |
| TM-35 | A token `realm` on another host. | Refused. No credentials sent (the server log shows none). |
| TM-36 | A redirect to another host, to HTTP, and a loop of six. | No `Authorization` sent cross-host, HTTP refused, loop stopped at 5. |
| TM-37 | Look for any option that disables TLS verification. | None exists. A self-signed server fails. |
| TM-38 | Helper names with `/`, `..`, `;` and spaces, a helper that hangs and one that prints 10 MiB. A helper in a relative `PATH` entry, an empty entry, a project directory, a world-writable directory and a group-writable directory of a group other than `admin`. A static login beside a helper that is absent. A static login in a registry auth file beside a configured helper. A helper behind a directory that group `admin` can write, owned by root or by the user, for a member of `admin`. A refused helper with a registry that demands a login, and with one that does not. | Names refused. The hang times out. Output capped. The planted helpers stay unstarted, and the refusal carries `credential-helper-untrusted` and a redacted directory. The static logins win and no helper starts. The `admin` layouts run. The first registry fails with the code, the second succeeds. |
| TM-39 | A release archive over HTTP, and with a wrong digest. | Refused before extraction. |
| TM-40 | A git repository with hooks, a submodule, a filter and a `file::` URL. | No code runs. The URL is refused. |
| TM-41 | An index with hostile text, an oversized body and a changed key. | Sanitised, refused, ignored. |
| TM-42 | `plugins publish` with no write scope. | Clear failure, no secrets printed. |
| TM-43 | A corpus of archives: `..`, absolute, drive letter, NUL, symlink, hardlink, device, FIFO, duplicate, case collision, setuid mode, overlong path, depth 33. | Each fails. The tree holds nothing outside staging, no links and no special modes. |
| TM-44 | A 1 GiB bomb, a ratio bomb and 100,000 entries. | Stopped at the limits with bounded disk and memory. |
| TM-45 | Kill the process during extraction. | The next run finds no entry and no staging leftovers. |
| TM-46 | Edit, add, delete and symlink-replace a file in a cached tree. | The load fails with exit 2 and the entry is evicted. |
| TM-47 | Check cache and lock permissions and temporary names. | 0700, random, exclusive. |
| TM-48 | A lockfile digest signed by an unpinned key, in CI. | Exit 2. The lockfile is unchanged. |
| TM-49 | A plugin whose dependency is source-only. | Refused. No build backend runs. |
| TM-50 | Edit one pin, add a package, swap a wheel, add a `.pth` file. | The trust hash changes and the spawn is refused. |
| TM-51 | Set a hostile `PIP_INDEX_URL`, `PYTHONPATH` and a `usercustomize`. | No effect on the build or the run. |
| TM-52 | Modify a file in the built environment after trust. | Refused at spawn. An edit that keeps the size and restores the modification time passes the spawn check and is refused by the full check in `status`, `doctor` and `trust` (section 6.6). |
| TM-53 | A `python` earlier on `PATH` than the managed one. | The managed interpreter runs. |
| TM-54 | A nested-quantifier pattern against `aaaa...` text. | Stops at the limit. |
| TM-55 | A limit hit in a normal run. | Exit 2, a diagnostic naming the rule. Never a pass. |
| TM-56 | A plugin with a 4,097-byte pattern and one with excessive rules. | The plugin is refused. |
| TM-57 | Patterns with verbs that raise limits and with callouts. | Limits stay. Callouts do nothing. |
| TM-58 | A document over the byte cap. | Reported at the cap, exit 2. |
| TM-59 | A plugin that tries to suppress another plugin's rules. | Ignored, with a diagnostic. |
| TM-60 | A revocation list, signed by the release key, that names the plugins key id. | Every first-party plugin and the first-party index fail verification with exit 2. A list signed by the plugins key is refused. |
| TM-61 | `core` as a statement id or namespace, a lock id, an index id or namespace, or a config plugin id. | Refused with exit 2. |

### 7.4 Implemented tests

These tests exist on `main` under `crates/slopvac-plugins/tests/`. The names in the table are test-name prefixes (`tm25_...`). Tests marked "open" belong to a bead that is not merged yet.

| Tests | File | Covers |
|---|---|---|
| H-tests (`h<N>_...`), TM-2, TM-3, TM-4, TM-5, TM-8 | `host.rs`, `teardown.rs`, `interrupt.rs` | P1 to P5, P7 to P9, P12 |
| TM-6, TM-9, TM-26, TM-28, TM-29 | `trust.rs` | P10, P13, K5 (revoked inputs), K7, K8, K9, O13 |
| TM-26 (spawn and status), TM-27, rotation | `revoke.rs` | K5, K6 |
| TM-22, TM-23, TM-24, TM-25, TM-30 | `sign.rs` | K1 to K4, K10 |
| TM-22, TM-26, TM-27, TM-39, TM-46, TM-48 | `sync.rs` | K1, K5, K6, N9, E8, E10 |
| TM-41 | `index.rs` | N11 |
| TM-33, TM-34, TM-35, TM-36, TM-38 | `oci.rs` | N1 to N4, N6 |
| TM-43, TM-44, TM-45, TM-46, TM-47 | `extract.rs` | E1 to E9 |
| TM-15, TM-16, TM-17, TM-18, H22 | `apply.rs` | O6 to O9 |
| TM-49, TM-50, TM-51, TM-52, TM-53 (`tm<N>_...`; the `live_` tests need uv and skip without it) | `pyenv.rs` | Y1 to Y6 |
| TM-5 limits, TM-4 on Linux (open) | `limits.rs` | P5, P6 |
| TM-11 | `sanitize.rs`, `host.rs`, `trust.rs` in `slopvac-plugins` | O1, P15 |
| TM-11, TM-12, TM-13 (the escaping functions) | `crates/slopvac-sanitize/src/lib.rs` | O1 to O4, P15 |

Tests with no implementation yet: TM-1 (named oversized frames, covered by H1 and H2), TM-7, TM-10, TM-14, TM-19 to TM-21, TM-31, TM-32, TM-37, TM-40, TM-42 and TM-54 to TM-59. TM-54 to TM-58 belong to `slopvac-8l4.3.20`. The rest wait for the bead that owns the threat. TM-11 to TM-13 pass at the component level (`slopvac-8l4.9.32`). The renderer and Action wiring is open in `slopvac-8l4.12.2` and `slopvac-8l4.13.6`, which add the end-to-end checks.

## 8. Work items from this model

Every work item of this model has a bead.

| Work | Bead | Status | Threats |
|---|---|---|---|
| Managed Python environments | `slopvac-8l4.9.31` | in progress | Y1 to Y7 |
| Safe application of edits and recipe plans | `slopvac-8l4.9.14` | closed | O6 to O9 |
| Plugin resource limits | `slopvac-8l4.9.13` | in progress | P5, P6 |
| Rotation and the signed revocation list | `slopvac-8l4.9.15` | in progress | K5, K6 |
| Shared sanitizer for plugin-origin text | `slopvac-8l4.9.32` | component done (crate `slopvac-sanitize`); renderer wiring open | O1 to O4, P15 |
| Archive extraction | `slopvac-8l4.9.3`, `crates/slopvac-plugins/src/extract.rs` | merged | E1 to E7 |
| Whole-run output budget | `slopvac-8l4.9.33` | open | P3 |

Other beads that this model depends on: `slopvac-8l4.9.16` (compiled first-party key), `slopvac-8l4.9.19` (credential values in the public trust hash), `slopvac-8l4.9.27` (pinned-commit git fetch), `slopvac-8l4.9.28` (executable set of a code plugin), `slopvac-8l4.9.29` (escaping in error messages) and `slopvac-8l4.3.20` (PCRE2 limits).

## 9. Residual risks

A residual risk stays after all controls above. The release notes and `slopvac plugins trust` output must state R1.

| ID | Risk | Why it remains | Impact |
|---|---|---|---|
| R1 | **Trust is not containment.** A trusted plugin runs with the user's rights. It can read files, open the network, start programs and modify the project. A plugin that declares `network: false` and connects anyway is undetectable. 3.0 ships no sandbox. | Sandboxing needs per-platform work (`sandbox-exec`, Landlock and seccomp, AppContainer). It is backlog. | High. Every hostile trusted plugin is full compromise of the account. |
| R2 | Resource limits are best effort. macOS lacks a reliable memory limit and process cap, and a daemonised descendant that escapes both the group and the tree walk survives. | Operating-system features differ. | Medium. A denial of service, or a persistent process. |
| R3 | Revocation is not instant. An offline or never-synced machine runs a revoked artifact until it fetches the list. | Offline mode is a feature (policies.md section 1.3). | Medium. |
| R4 | A provider plugin can send any text it receives. The dry run shows the payload and says nothing about what the plugin does with it. | The host cannot observe a plugin's network use. | Medium to high for private documents. |
| R5 | `SLOPVAC_TRUST` in CI trusts whatever the workflow names. A pull request that changes the workflow can name its own hash. | Review of CI configuration is the project's job. | High in an unprotected repository. |
| R6 | A local process of the same user can write the trust store, cache or environments, and can swap files between check and use, or swap a destination, its parent or a temporary file during a `slopvac-fs` write. | The user's rights equal the attacker's. The design only raises the cost. | High, but needs prior local compromise. |
| R7 | The first key pin is trust on first use. A user who confirms a hostile fingerprint trusts it. | The user has to decide somewhere. The fingerprint display and the namespace binding reduce the risk. | Medium. |
| R8 | The release key is the single root of trust. Replacing a lost or leaked release key takes a new slopvac release. A leaked plugins key is revocable by the release key (K13). | A compiled key changes only with a release. | High, with low likelihood. |
| R9 | Prompt injection cannot be solved at the host. The host only cuts the paths from provider output to edits and verdicts. | Model behavior is outside slopvac. | Medium. |
| R10 | A signed plugin can contain misleading or harmful rules. A signature identifies a publisher and does not judge content. | Review of content is outside the supply chain. | Low to medium. |
| R11 | **Release freshness is unauthenticated.** `slopvac self update` and the update notice read the latest version from unsigned registry JSON: the GitHub releases API, or the PyPI JSON for wheel installs. An attacker who controls that answer (T2) can freeze or delay upgrades, or name an older signed release that is still newer than the running one. The attacker cannot install anything below the running version or anything the release key did not sign: the update pipeline refuses a version that is not newer than the running one or is a prerelease (`crates/slopvac-update/src/pipeline.rs`), and the minisign check against the compiled-in `keys/slopvac-release.pub` binds the archive to the exact asset and version (`crates/slopvac-update/src/verify.rs`). | A signed, expiring release manifest needs release-pipeline work that 3.0 does not include. Follow-up: `slopvac-8l4.13.46`. | Medium. A user can stay on a version with a known flaw while an attacker withholds the fix, and nothing in the client shows it. |
| R12 | **A repository writer can overwrite the published revocation list.** A writer can dispatch a modified branch of `revocations.yml`, which holds `packages: write`, and overwrite `ghcr.io/slopvac/slopvac-revocations:latest`. The writer can deny service but cannot forge a list: clients verify against the compiled-in release key (K6), so an invalid list fails verification and online syncs exit 2 until a maintainer publishes a valid list. The signing key is separate: it lives in the `revocations` environment, which deploys from `main` only. | GitHub grants package write access to workflows from any branch of the repository, and the registry cannot require a signature on push. Limiting write access to the repository is the project's job. | Medium. Online syncs fail until a valid list is pushed, and the failure is visible. |
| R13 | **`slopvac report --gh --yes` runs the first `gh` on `PATH`.** A directory that an attacker controls and that comes early on `PATH` can supply a fake `gh`, which then receives the issue title and the path of the body file, so it can read the issue body, and can use the login of the real `gh`. The search does not yet use trusted directories only. | The trusted-directory rule for helper programs is not merged. This is an accepted residual risk. Moving the lookup to trusted directories is follow-up work after that rule merges. The user already controls `PATH`, and the preview shows the argument list before anything runs. | Medium. |
| R14 | An attacker who can delete files in the install directory can unlink an active `slopvac.update.lock` and break the serialization of `slopvac self update`. The lock cannot defend against that attacker. | The same attacker can replace the binary itself, so write access to the install directory is already full compromise of the installed `slopvac`. The lock guards against concurrent updates, not against a hostile directory. | High, but needs prior write access to the install directory. |
| R17 | **A same-user process that changes the checkout during a migrate run can defeat migrate's checks.** `slopvac migrate --write` and `--restore` defend against a hostile checkout that stays the same while migrate runs. Migrate follows no symbolic link and refuses a file with more than one hard link. It binds its restore record, in the per-user cache, to the checkout's identity and to one migration instance. It treats a target as its own only when both the object's identity and its digest match, and otherwise it fails closed. A process running as the same user can still change the tree between a check and the next step, for example by swapping a file in migrate's holding directory before an unlink, or by moving a parent directory out of the project. | That process already has the user's write access to the tree, so it can make the same changes without migrate. Closing the window needs a lock or a staging area that the checkout cannot reach. Revisit if migrate gains one. | Low. The attacker already needs to run code as the user. |

## 10. Review sign-off

The independent review needs to record the following before `slopvac-8l4.9.2` and `slopvac-8l4.9.6` close:

1. Each threat in section 6 has a built control or an accepted residual risk, and a passing test.
2. The signature statement format of section 5.1 is fixed in `slopvac-8l4.9.5` and matches the verifier.
3. The key authority of section 5.2 holds against a rebuilt index in a live test (TM-22).
4. The open decisions are settled in this file: the Python environment verification cost (Y5) and the revocation staleness window (R3). The signature layer media types are fixed in section 5.1.
5. Every open bead in section 8 has a schedule, and a dropped item has a recorded reason.
6. R1 is visible in user-facing text.
