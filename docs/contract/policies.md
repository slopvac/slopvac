# Slopvac 3.0 policies: network, accessibility and API lifecycle

Status: contract for 3.0. Bead `slopvac-8l4.2.12`. Plan: [slopvac-3.md](../design/slopvac-3.md).

This document sets three policies that the 3.0 binary, its schemas, and its plugin protocol must follow.

Paragraphs and bullets labeled **2.x behavior** describe `packages/slopvac-lint`, and each names the source file. Every other statement is a 3.0 requirement, and an owning bead follows it. The release-readiness gate (`slopvac-8l4.13.9`) lists this document and blocks on its conformance tests (section 4). An unowned requirement is a gap to close before the gate.

"Must" is a requirement on the 3.0 binary. "Network" means any connection outside the local machine, including DNS lookups and registry or API calls.

## 1. Network and privacy

### 1.1 Principles

1. `lint`, `validate`, `rules`, `explain`, `prime`, `reference`, and `claims` read local files only. The one exception is the background update check (section 1.2), which the opt-outs in section 1.3 disable.
2. Every network feature is a named command or hook in the table in section 1.2. No other code path opens a connection.
3. Document text, finding text, file paths, and rule hit counts stay local. The update-check request is the only usage-like traffic, and it carries no document data. Slopvac has no telemetry.
4. A failed or refused connection never changes a lint verdict. It prints an error, and it exits 2 (incomplete) only when the user asked for the network operation.
5. Slopvac has no prompts, so consent is an explicit command or flag. A guard such as `--submit` binds a command and cannot tell who ran it. The prime text therefore tells agents to get the user's approval of the previewed output first.

### 1.2 Inventory of network features

| Feature | 2.x behavior | 3.0 requirement | Owner |
|---|---|---|---|
| Update check | A detached process asks `https://pypi.org/pypi/slopvac/json` for the latest version and caches the result for 24 hours (`update_notice.py`). Any command except `prime` may start it. Machine-readable output or a report file prevents it (`cli.py`). It runs only when stderr is a TTY. | Check the registry that matches the install method (`update-informer`). Only `lint`, `doctor`, `prime`, and `self update` may start it. Cache the result for 24 hours. Show it in `prime`, in JSON `notices`, and in `--version --json`, so agents see it. | `slopvac-8l4.12.4` |
| `self update` | None. | Runs only for installs from the slopvac installer. It downloads the release archive, verifies the minisign signature, and then replaces the binary. | `slopvac-8l4.12.4`, `slopvac-8l4.13.3` |
| OCI pack and plugin fetch | None. 2.x has no packs. | `plugins sync`, `plugins update`, and `pack publish` pull or push OCI artifacts. The client is hand-written on `ureq`. Initial support covers the documented GHCR flows (anonymous and basic tokens). `docker_credential` helpers extend this to other registries within the tested scope of the owning bead. | `slopvac-8l4.9.2`, `slopvac-8l4.9.3`, `slopvac-8l4.9.9` |
| Release-archive packs | None. | `plugins sync` downloads a pack or plugin archive over HTTPS and checks the digest that the lockfile pins. | `slopvac-8l4.9.3` |
| Git packs | None. | A pack pinned to a git commit clones during `plugins sync` only. | `slopvac-8l4.9.3` |
| Outdated check | None. | `plugins outdated` lists newer digests and versions from the pinned sources. | `slopvac-8l4.9.3` |
| Pack index search | None. | `packs search` reads the published index over HTTPS. | `slopvac-8l4.9.8` |
| Issue submission | `prime report` prints a workflow that tells an agent to run `gh issue create` after the user approves the exact post (`guidance.py`). The `gh` program connects. | `slopvac report` reads a token from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token` and posts the issue. Without a token it opens a prefilled issue-form URL. Both paths need the consent step in section 1.5. | `slopvac-8l4.12.6` |
| Model-provider hooks | None. | A plugin that implements the model-provider hook may contact a model service. Slopvac core does not. Consent rules apply (section 1.5). | `slopvac-8l4.9.6`, `slopvac-8l4.11.3` |
| LSP server | None. | `slopvac lsp` talks to the editor over stdio and opens no connection. | `slopvac-8l4.12.7` |
| Code plugins in general | None. | A plugin process is a separate program, and 3.0 leaves its network access unsandboxed. The trust command (section 1.6) is the control. The threat model records the residual risk. | `slopvac-8l4.9.4`, `slopvac-8l4.9.11` |

Downloads at install time sit outside the binary. In this repository, `scripts/install_vale.py` downloads Vale for development, and 3.0 removes Vale (`slopvac-8l4.16.4`).

### 1.3 Offline mode and opt-outs

**2.x behavior.** The update check stops when any of these is true (`update_notice.py`, `_disabled_by_environment` and `_enabled`, and `cli.py`, `_machine_output`):

- the `CI` variable is set, with any value.
- `NO_UPDATE_NOTIFIER`, `SLOPVAC_NO_UPDATE_CHECK`, or `DO_NOT_TRACK` has a non-empty value.
- stderr is not a TTY.
- the command is `prime`, the format is not `text`, or `--out` or `--open` is present.

The environment variables above are the only switches in 2.x. Under `CI` or the other opt-outs, 2.x shows no notice either.

**3.0 requirement.** Offline mode blocks every row of section 1.2. These switches turn it on:

| Switch | Effect |
|---|---|
| `--offline` (global flag) | Offline for this run. |
| `SLOPVAC_OFFLINE=1` | Offline for every run in the environment. |
| `offline = true` under `[network]` in `slopvac.toml` | Offline for the project. |

Offline is sticky. If any source sets it, no other source can unset it, and no flag turns it off. This departs from the layer order of the plan, where a higher layer overrides a lower one.

In offline mode:

- A command that needs the network exits 2 with the code `offline-network-needed` and the message `offline: <command> needs the network`. It does not retry or fall back.
- Packs that the lockfile pins and that sit in the local cache load normally.
- A pack missing from the cache exits 2 with the code `offline-pack-missing`. The message names the pack and the digest.
- `plugins sync` succeeds from the cache alone. It verifies digests against `.slopvac/packs.lock` and reports each missing pack.

These opt-outs apply to the update check only:

| Switch | Effect |
|---|---|
| `NO_UPDATE_NOTIFIER`, `SLOPVAC_NO_UPDATE_CHECK`, `DO_NOT_TRACK` | Same as 2.x. |
| `update_check = false` under `[network]` | Project-level opt-out. |
| `CI` | No background check and no notice, as in 2.x. |

Owners: `slopvac-8l4.12.1` for the global flag and environment, `slopvac-8l4.4.2` for the config keys, `slopvac-8l4.12.4` for the update check.

### 1.4 CI defaults

- Slopvac detects CI from the `CI` variable, as 2.x does.
- In CI the update check stays off.
- No code path prompts, in CI or anywhere else.
- `plugins sync` in CI runs from the lockfile. It fails with exit 2 when the lockfile is missing or a digest differs, and it leaves the lockfile as it found it.
- A GitHub Action run that needs packs runs `plugins sync` as a separate, named step. The lint step runs with `--offline`.

Owners: `slopvac-8l4.9.3` for lockfile behavior, `slopvac-8l4.13.6` for the Action.

### 1.5 Consent

Two features need consent beyond installing slopvac: issue submission and model-provider hooks.

**Issue submission.**

- `slopvac report` prints the exact title, labels, and body it would post, then exits. Posting needs `--submit`.
- The body omits local paths, file names, user names, host names, repository names, and organization names. The body includes document text only when the user passes `--include-text <file>`, and the preview shows that text.
- Environment fields come from the CLI, never from environment variables, as `CONTRIBUTING.md` requires for agent reports.
- 2.x already requires the agent to get the user's approval of the exact post (`guidance.py`, the `prime report` workflow). 3.0 keeps that rule in the prime text and adds the command-level guard above.

**Model-provider hooks.**

- A model provider is a plugin. It stays off until the user runs `slopvac plugins trust` for it and names it in `slopvac.toml` under `[review] provider`.
- Slopvac sends a provider only the text that the review type needs: changed claims for fact checking, the changed passages for `update`, and the whole document for `rewrite`. `slopvac review --dry-run` prints the exact payload without calling the provider.
- During `lint`, every provider stays idle.
- In offline mode no provider call happens, even for a trusted provider.
- The reviewer model must differ from the author model. Slopvac cannot verify this for a provider it does not control. The check compares the model names that the config and the provider report.

Owners: `slopvac-8l4.12.6`, `slopvac-8l4.9.6`, `slopvac-8l4.11.3`.

### 1.6 Trust and signatures

- The plugin host refuses to load a code plugin until its content hash is trusted. A hash change revokes trust.
- `slopvac plugins trust` records trust. `sync`, `update`, and flags such as `--yes` never record it.
- CI has one explicit exception: the `SLOPVAC_TRUST` environment variable, which `slopvac-8l4.9.4` defines. A person writes it into the CI configuration, and it names what to trust. It is configuration, not a prompt, and no tool sets it on its own.
- OCI artifacts carry minisign signatures as manifest layers. Slopvac verifies the signature before extraction, and a missing or invalid signature makes `sync` fail with exit 2.
- Release archives verify against the release signature (`self update`).
- Local folders and git commits carry no signature. A git pin names a commit, and the trust hash covers the content.

Owners: `slopvac-8l4.9.4`, `slopvac-8l4.9.5`, `slopvac-8l4.9.11`.

### 1.7 Logging and redaction

**2.x behavior.** Slopvac has no logging framework and no log file. Two redactions exist:

- Slopvac stages `--stdin` input in a temporary file. Every report shows the path as `<stdin>`, and the temporary directory name never reaches output (`pipeline.py`, `redact_stdin`). Tests in `packages/slopvac-lint/tests/test_cli.py` cover text, JSON, and error output.
- The update-check cache stores the check time and the latest version string.

**3.0 requirement.**

- Logging uses `tracing` and stays off until `SLOPVAC_LOG` is set. Log output goes to stderr, and no log file exists.
- Logs never contain document text, finding text, matched spans, front matter values, or terms.
- Logs record document paths relative to the project root. A path outside the root appears as `<external>/<file name>`. Stdin appears as `<stdin>` everywhere, including logs, panics, and error chains.
- Logs never contain tokens, `Authorization` headers, or URL credentials. Before it logs a URL, slopvac strips the user-info part.
- Rule IDs, pack IDs, digests, durations, and counts may appear in logs.
- Panic and error output follow the same rules.
- `SLOPVAC_LOG=trace` lifts none of these rules.

Owners: `slopvac-8l4.12.1` for the logging setup, `slopvac-8l4.12.9` for stdin.

### 1.8 Data written to disk

- Caches and state go to the per-user directories that `etcetera` reports. `SLOPVAC_CACHE_DIR` overrides the cache location, as in 2.x (`vale_cache.py`).
- Slopvac writes files atomically and takes a lock for shared state (`tempfile` persist and `fs4`).
- Slopvac writes only to the cache directory, the project's `.slopvac/` directory, and paths the user names.

Owners: `slopvac-8l4.9.3`, `slopvac-8l4.9.4`.

## 2. Accessibility

### 2.1 Plain and no-color output

**2.x behavior.**

- Text output goes through a `rich` console. Rich drops styling when stdout is not a terminal, so piped output carries no escape codes (`cli.py`, `_console`). `test_cli.py` tests this.
- `lint --no-color` removes color codes. A test checks that no color code appears. In a forced terminal, bold and italic codes remain, and no test covers that.
- JSON, SARIF, and GitHub annotations contain no terminal escape codes.
- 2.x ignores the `NO_COLOR` variable. `_console` passes `no_color=False`, which overrides the variable that `rich` would read. The 2.x line ends at release 2.18.1, so the defect stays in 2.x.

**3.0 requirement.** Slopvac resolves color once, in this order:

1. `--color never|always|auto` (default `auto`) wins when given.
2. A non-empty `NO_COLOR` means `never`.
3. A non-empty `CLICOLOR_FORCE` or `FORCE_COLOR` means `always`.
4. Otherwise, color is on if stdout is a terminal and `TERM` is not `dumb`.

`--no-color` stays as an alias for `--color never`.

When color is `never`, output also drops bold, italic, underline, and cursor controls. Tables use ASCII borders when `TERM=dumb` or when the locale is not UTF-8.

Owners: `slopvac-8l4.12.1` for the flags, `slopvac-8l4.12.2` for the renderers.

### 2.2 Text that does not depend on sight

- Words carry every signal, and color only reinforces them. Every severity, verdict, and status appears as a word.
  - 2.x text output prints `PASS`, `FAIL`, and `UNCHECKED` (`pipeline.py`).
  - 2.x HTML prints `PASS`, `FAIL`, the severity word of each finding, and a heading that counts the checks that did not run (`html.py`).
- Each finding prints on its own line in the form `path:line:column severity rule-id message`, so a screen reader reads the location first.
- Text output has no spinners, progress bars, or animation. Progress goes to stderr. It appears only if stderr is a terminal and color is not `never`.
- Symbols and emoji carry no meaning. Box-drawing characters are decoration, and each table row also reads in order as plain words.
- When color is `never` and stdout is not a terminal, the renderer adds no alignment padding and inserts no hard line breaks into messages.

Owner: `slopvac-8l4.12.2`.

### 2.3 Accessible HTML report

**2.x behavior** (`html.py`):

- The page sets `lang="en"`, has a `<title>`, and puts content in `<main>`.
- Tables use `<th>` header cells. File details use `<details>` and `<summary>`.
- The verdict and the severity of each finding appear as text.
- The stylesheet respects `prefers-color-scheme`. Computed from the stylesheet variables with the WCAG relative-luminance formula, the status and muted text colors reach at least 4.87:1 against their panel in the light scheme. In the dark scheme they reach at least 5.26:1.
- The `_e` function escapes text from user content or findings: paths, notes, messages, rule IDs, replacements, the timestamp, and the version. The verdict and band words come from constants, and the statistics are numbers, so nothing escapes them.

**3.0 requirement.** The 3.0 report keeps those properties and adds:

- `scope="col"` or `scope="row"` on table headers, and a `<caption>` on each table.
- One `<h1>`, and headings that skip no level.
- A "Skip to findings" link as the first focusable element.
- Visible keyboard focus on every interactive element, with at least 3:1 contrast.
- A `prefers-reduced-motion` media query. The report has no animation.
- Contrast of at least 4.5:1 for body text and 3:1 for large text and UI borders, in both schemes.
- No JavaScript and no remote resources (fonts, scripts, or images).

An automated checker (axe-core) finds only part of the WCAG 2.2 AA failures. The release checklist adds a manual keyboard and screen-reader pass.

Owner: `slopvac-8l4.12.2`.

### 2.4 Language

- 3.0 messages, rule explanations, and generated pages are English only. `slopvac-8l4.18.2` tracks other languages after 3.0.
- Message text is not an interface. Consumers branch on rule IDs, error codes, and exit codes (section 3.5).
- The HTML report sets `lang="en"`.

Owners: `slopvac-8l4.12.1` for CLI messages, `slopvac-8l4.4.7` for config diagnostics, `slopvac-8l4.12.2` for the HTML report.

## 3. API lifecycle

### 3.1 Versioned surfaces

| Surface | Version field | Where it lives | Schema | Owner |
|---|---|---|---|---|
| Lint JSON report | `schema_version` (integer) | Report root | `lint-report` | `slopvac-8l4.2.2` |
| Config file | `schema_version` (integer) | `slopvac.toml` root | `config` | `slopvac-8l4.2.5` |
| Front matter block | `v` | `slopvac:` key | `frontmatter` | `slopvac-8l4.2.5` |
| Rule, category, style, genre, and terms files | `schema_version` (integer) | File root | One per artifact | `slopvac-8l4.2.3`, `slopvac-8l4.2.4` |
| Pack manifest | `slopvac_api` (integer) | `slopvac-pack.toml` | `pack-manifest` | `slopvac-8l4.2.4` |
| Lockfile | `schema_version` (integer) | `.slopvac/packs.lock` | `packs-lock` | `slopvac-8l4.2.4` |
| Plugin protocol | `protocolVersion` (integer) | `initialize` request and response | `plugin-protocol` | `slopvac-8l4.2.6` |

The shared definitions are `SchemaVersion` and `ApiVersion` in `schemas/v3/common.schema.json`. The scaffold bead `slopvac-8l4.2.19` freezes their shape, and `slopvac-8l4.2.12` owns their meaning (section 3.2).

### 3.2 `schema_version` and `slopvac_api`

**2.x behavior.** `LintReport.schema_version` defaults to 1. The docstring in `report.py` states the rule: it changes only when a key is removed or retyped, and adding a key does not bump it. The report has a separate `version` key for the tool version. `doctor --format json` has its own `schema_version` of 1 (`doctor_report.py`). 2.x config files have no version field, and `extra="forbid"` rejects unknown keys (`config.py`). The 2.x function `update_notice.upgrade_hint` returns an install-method command such as `uv tool upgrade slopvac`, `pipx upgrade slopvac`, or `pip install -U slopvac`.

**3.0 requirement.**

- `SchemaVersion` is a positive integer. It counts breaking changes to one artifact's shape and has no minor or patch part.
- Every root instance of a versioned artifact carries `schema_version` with the constant 3. Lint JSON jumps from 1 to 3 so that all artifacts share one number at first. The migration guide states this jump. A later breaking change raises only the number of the artifact it affects, so numbers can diverge after 3.0.
- Changes within one `schema_version` are additive only.
  - Allowed: a new optional field, a new optional section, and a new value of an output-only enum that the schema marks as extensible.
  - Not allowed: removing or renaming a field, changing a type, tightening a constraint, or giving a field a new meaning.
- A schema's `$id` never changes within a version. The published URL has the form `https://srobroek.github.io/slopvac/schemas/v3/<artifact>.schema.json`.
- `ApiVersion` is a positive integer for the pack and plugin extension API. The first pack manifest declares `slopvac_api = 1`. It names the host capabilities a pack needs. `schema_version` names the shape of one file. The two are independent.
- The binary version is independent of both. A release that adds optional fields keeps `schema_version`. `slopvac --version --json` lists the binary version, each supported `schema_version`, and each supported `slopvac_api`.
- A config file without `schema_version` is a 2.x config and is rejected (section 3.7). Any other 3.0 artifact without a version is an error.
- A version higher than the binary supports is an error. The message names the file, the version found, and the versions supported. It also prints the upgrade command for the install method, built as `upgrade_hint` builds it in 2.x.

Owners: `slopvac-8l4.2.12` for the semantics, `slopvac-8l4.4.5` for the config version check.

### 3.3 Unknown-field handling

The rule depends on who wrote the file.

| Reader | Writer | Unknown field | Reason |
|---|---|---|---|
| Slopvac | A user (config, front matter, rules, packs) | Error with file, line, and column, plus the nearest valid key | A misspelled key would silently disable a setting. 2.x already forbids unknown keys. |
| Slopvac | A plugin (protocol messages) | Ignored, logged at debug level | A plugin can be newer than the host within one API version. |
| A consumer of slopvac output | Slopvac | The consumer must ignore it | Additive changes do not bump `schema_version`. |
| A plugin | Slopvac (protocol messages) | The plugin must ignore it | Same reason. |

Authors who attach their own data to a user-written file put it under a key with the prefix `x-`. Slopvac ignores `x-` keys wherever it forbids unknown keys and never gives one a meaning. Each artifact schema records this rule in its `description`.

The schema marks which output enums can gain values. Consumers must tolerate new values of those enums.

Owners: `slopvac-8l4.2.3`, `slopvac-8l4.2.4`, `slopvac-8l4.2.5`, `slopvac-8l4.2.6`.

### 3.4 Capability negotiation

The plugin protocol follows LSP. The `initialize` request carries the host's `protocolVersion`, its supported hook points, and its feature flags. The response carries the plugin's `protocolVersion`, the hooks the plugin implements, and the features it needs.

- The host picks the highest protocol version that both sides support. With no common version, the host does not load the plugin and prints both version lists.
- The host calls only the hooks that the plugin declared.
- If the host lacks a required feature, the load stops with an error that names the feature. The host skips an optional feature it lacks.
- Both sides ignore unknown capability names (section 3.3).
- A data pack has no handshake. The host compares `slopvac_api` with the range it supports. It loads the pack only if the value is in range.

The plugin protocol bead defines the message shapes.

Owners: `slopvac-8l4.2.6`, `slopvac-8l4.9.6`, `slopvac-8l4.9.1`.

### 3.5 Error codes

**2.x behavior.** The CLI exit codes are 0 (pass), 1 (a gate failed), and 2 (the run was incomplete) (`contract.py`). Exit 2 means the result cannot establish a pass. 2.x messages have no stable codes.

**3.0 requirement.**

- The exit codes keep their meaning. Slopvac uses no other exit code.
- Every error and notice has a stable `code`: a lowercase kebab-case string such as `config-unknown-key` or `pack-api-unsupported`. The code appears in brackets in text output, in JSON `notices` and errors, and in the generated documentation.
- No code is reused, and no code changes meaning. A code that stops occurring stays in the documentation, marked retired.
- Plugin protocol errors use the JSON-RPC 2.0 error object. Its numeric `code` follows the JSON-RPC ranges, and `data.slopvac_code` carries the string code. The protocol bead defines the numeric range.
- Message text can change in any release. Codes, rule IDs, and exit codes form the interface.

Owners: `slopvac-8l4.2.6` for the protocol, `slopvac-8l4.4.7` for config diagnostics, `slopvac-8l4.12.1` for the CLI.

### 3.6 Breaking changes

3.0 carries no deprecation machinery. A breaking change takes effect at a major version, and the release notes list it.

- Minor and patch releases of 3.x make additive changes only (section 3.2). A removal or rename needs a major version.
- Warning periods, deprecated aliases, and flags that emit old output formats do not exist. Slopvac 3.0 rejects removed flags, options, hooks, and rule IDs with an error and exit 2.
- Pack `slopvac_api` and plugin `protocolVersion` follow the same rule. The host loads a pack or plugin whose version it supports, as section 3.4 describes. A pack or plugin outside that range stays unloaded, and the error names both versions. No grace period applies.

Owners: `slopvac-8l4.12.1`, `slopvac-8l4.9.1`.

### 3.7 2.x surfaces at 3.0

3.0 invalidates the 2.x surfaces at release. Slopvac has no 2.x compatibility mode.

- A config file with no `schema_version`, or with a 2.x value, is rejected with exit 2. This covers `.slopvac.toml`, `slopvac.toml`, and `[tool.slopvac]` in `pyproject.toml`. The error has the code `config-legacy-version`, names the file, and points to `slopvac migrate`.
- Slopvac lints nothing while it holds a 2.x config. No partial run happens.
- `slopvac migrate` is the only path from 2.x. It previews the changes, rewrites the config with comments kept, updates directive IDs and steering blocks in documents, and compares findings before and after.
- The mapping covers every 2.x setting, including negated globs, category budgets and weights, per-path locale, vocabulary files, and guidance controls. A setting with no 3.0 equivalent produces a `config-legacy-dropped` error in `migrate` and never disappears silently.
- A 2.x flag, option, or hook that 3.0 removes fails with an error and exit 2. The compatibility matrix (`slopvac-8l4.2.9`) lists each one with the disposition `remove`.
- Lint JSON from 3.0 has `schema_version` 3 only. No flag emits the 2.x format.

Owners: `slopvac-8l4.4.5` for the version check, `slopvac-8l4.2.5` for the mapping table, `slopvac-8l4.16.1` for `migrate`, `slopvac-8l4.16.2` for `prime migrate`.

### 3.8 SDK support

- The Python SDK implements one `protocolVersion` per SDK major version. It supports the protocol versions that the matching host release supports (section 3.4).
- The SDK ships a conformance suite. A plugin in any language can use it, and the example plugin must pass it.
- The SDK validates messages against the published `plugin-protocol` schema, so a protocol mistake shows up in the plugin's own tests.
- SDK releases follow semantic versioning. A minor release can add optional capabilities. A protocol-incompatible change needs an SDK major release (section 3.6).
- An SDK in another language builds on the published JSON Schemas.

Owners: `slopvac-8l4.9.7`, `slopvac-8l4.19.1`.

## 4. Conformance tests

The 3.0 binary must pass these tests. Each runs the installed binary with an isolated home and cache directory. `slopvac-8l4.13.9` blocks the release until all pass.

| ID | Test | Owner |
|---|---|---|
| N1 | With the update check disabled, `lint`, `validate`, `rules`, `explain`, `prime`, `reference`, and `claims` each finish under a network sandbox that fails any connection. | `slopvac-8l4.12.1` |
| N2 | `--offline`, `SLOPVAC_OFFLINE=1`, and `[network] offline = true` each make `plugins sync` exit 2 with the code `offline-pack-missing` when the cache lacks a pack. A command with no network path exits 2 with `offline-network-needed`. | `slopvac-8l4.9.3` |
| N3 | `SLOPVAC_OFFLINE=1` with `[network] offline = false` in the config stays offline. The CLI help lists no flag that turns offline mode off. | `slopvac-8l4.12.1` |
| N4 | With a populated cache and a lockfile, `plugins sync --offline` succeeds and verifies digests. | `slopvac-8l4.9.3` |
| N5 | No update check starts under `CI`, `NO_UPDATE_NOTIFIER`, `SLOPVAC_NO_UPDATE_CHECK`, `DO_NOT_TRACK`, `update_check = false`, or `--offline`. The test extends the 2.x test `test_environment_suppression_prevents_refresh_and_notice`. | `slopvac-8l4.12.4` |
| N6 | Machine formats keep stdout free of update notices, and JSON carries the notice in `notices`. | `slopvac-8l4.12.4` |
| N7 | `plugins sync` in CI exits 2 on a missing lockfile or a digest mismatch and leaves the lockfile unchanged. | `slopvac-8l4.9.3` |
| N8 | `sync` and `update` never record trust, with or without `--yes`. Only `plugins trust` and `SLOPVAC_TRUST` do. | `slopvac-8l4.9.4` |
| N9 | `slopvac report` without `--submit` sends zero requests and prints the full body. Against a mock server, `--submit` posts exactly the previewed body. | `slopvac-8l4.12.6` |
| N10 | The issue body for a run in a directory with a unique marker path and user name contains neither. | `slopvac-8l4.12.6` |
| N11 | A review run with a model provider and `--dry-run` leaves the provider uncalled and prints the payload. A `lint` run leaves every provider uncalled. | `slopvac-8l4.11.3` |
| N12 | A model provider that is untrusted or absent from the config never starts. | `slopvac-8l4.9.6` |
| R1 | With `SLOPVAC_LOG=trace`, a run over a document with a unique sentinel string logs no copy of the sentinel from the text, matches, front matter, or terms. | `slopvac-8l4.12.1` |
| R2 | The same run over stdin logs `<stdin>` and no temporary path, in logs, errors, and panics. | `slopvac-8l4.12.9` |
| R3 | Paths inside the project root log relative to it. Paths outside log as `<external>/<file name>`. | `slopvac-8l4.12.1` |
| R4 | A token in `GH_TOKEN` and credentials in a registry URL appear in no log, error, or notice. | `slopvac-8l4.9.2` |
| C1 | With stdout piped, no format contains an escape byte. | `slopvac-8l4.12.2` |
| C2 | `NO_COLOR=1`, `--color never`, and `--no-color` each remove all escape codes, including bold and italic, even when `FORCE_COLOR=1` is set. | `slopvac-8l4.12.1` |
| C3 | `--color always` forces color on a pipe, and `--color` wins over `NO_COLOR`. | `slopvac-8l4.12.1` |
| C4 | Every severity and verdict in text output appears as a word. A snapshot test strips all escape codes and compares. | `slopvac-8l4.12.2` |
| C5 | `TERM=dumb` output uses ASCII borders and no cursor controls. | `slopvac-8l4.12.2` |
| H1 | The HTML report passes axe-core with no violations, has `lang="en"`, one `<h1>`, table captions and `scope`, and no remote resource. | `slopvac-8l4.12.2` |
| H2 | The report stylesheet meets the contrast ratios in section 2.3, computed from its variables in both schemes. | `slopvac-8l4.12.2` |
| V1 | Every published schema accepts its conformance instances and rejects the invalid ones. | `slopvac-8l4.2.19` |
| V2 | An artifact with a `schema_version` above the supported range fails with the code, the versions, and the upgrade command. | `slopvac-8l4.4.5` |
| V3 | A user-written file with an unknown key fails with file, line, and column, and an `x-` key passes. A plugin message with an unknown field is ignored. | `slopvac-8l4.2.5`, `slopvac-8l4.2.6` |
| V4 | A handshake with no common protocol version, a missing required feature, or an unknown capability produces the behavior in section 3.4. | `slopvac-8l4.9.6` |
| V5 | A pack with an unsupported `slopvac_api` stays unloaded, and the error names both versions. | `slopvac-8l4.9.1` |
| V6 | Every error code in the generated docs is unique and kebab-case, and a later release keeps every code of the earlier one. | `slopvac-8l4.19.1` |
| V7 | A config without `schema_version`, or with a 2.x value, exits 2 with `config-legacy-version` and a pointer to `slopvac migrate`, and lints nothing. | `slopvac-8l4.4.5` |
| V8 | Each removed 2.x flag, option, and hook fails with an error and exit 2. No 2.x JSON format flag exists. | `slopvac-8l4.2.9` |
| V9 | The Python SDK example plugin passes the conformance suite. | `slopvac-8l4.9.7` |
| V10 | Each 2.x setting listed in section 3.7 maps in `slopvac migrate`, or produces `config-legacy-dropped`. | `slopvac-8l4.2.5`, `slopvac-8l4.16.1` |

The Python 2.x tests that cover current behavior sit in `packages/slopvac-lint/tests/`:

- stdin redaction in text, JSON, and error output: `test_cli.py`.
- update-check opt-outs and machine-format suppression: `test_update_notice.py`.
- piped text output without escape codes, forced-terminal color codes, and `--no-color` without color codes: `test_cli.py`.

`NO_COLOR` has no 2.x test, because 2.x ignores the variable (section 2.1).

## 5. Follow-up for bead owners

- `slopvac-8l4.12.6` adds the preview and `--submit` behavior from section 1.5.
- `slopvac-8l4.13.6` is the Action bead. Its description lists reviewdog, SARIF, and CodeClimate, so it needs the `plugins sync` step and the `--offline` lint step from section 1.4.
- `slopvac-8l4.9.8` is named "Pack index and slopvac packs search". The plan lists `search` under `plugins`, so the plan or the bead needs one name for the command.
- `slopvac-8l4.12.4` confirms which commands may start the update check (section 1.2).
