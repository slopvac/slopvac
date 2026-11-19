# Engineering standards

This page lists the requirements every change to the Rust code in `crates/` must meet. Reviewers, human and agent, check a change against it.

- A rule marked "target: after `<bead>`" is not met by the whole code base yet. New code follows it. Existing code moves when the named bead lands.
- A rule without a marker holds now.
- To deviate from a rule, follow [How to request a departure](#how-to-request-a-departure).

Each rule gives a source. A repository source is a link. A source written as skill `<id>` is a coding-agent standard, not a repository file. Skill ids refer to the team's OpenCode plugin standards.

## Errors

- Libraries define one `thiserror` enum per module and a `Result` alias. Derive `Error`, do not write `Display` by hand, and size each enum to its operation. Exception: a `Display` that must sanitize text. Why: callers match on variants, and the derive keeps messages and sources in sync. Target: after `slopvac-8l4.3.50` (engine) and `slopvac-8l4.2.32` (contract and plugins), because `slopvac-engine`, `slopvac-plugins` and `slopvac-rules` still use error structs or hand-written `Display`. Sources: [architecture record, decision 10](design/0001-slopvac-3-architecture.md#10-error-and-diagnostic-rendering-thiserror-anyhow-annotate-snippets), skill `lang-rust-libraries`.
- The binary reports failures through the typed `CliError`. Each error carries a stable kebab-case code. Text output renders `error[<code>]: <message>`. Why: codes are part of the interface and message text is not. Sources: [`errors.rs`](../crates/slopvac/src/errors.rs), [policies, error codes](contract/policies.md#35-error-codes).
- Take every code from one registry. No free-string code at a call site. Target: after `slopvac-8l4.21.6`.
- Never reuse a code and never change its meaning. A retired code stays in the generated docs, marked retired. Source: [policies, error codes](contract/policies.md#35-error-codes).
- Use only exit codes 0 (pass), 1 (a gate failed) and 2 (run incomplete). Treat exit 2 as incomplete. A panic under `panic = "abort"` exits with another code, so keep panics out of library code. Source: [policies, error codes](contract/policies.md#35-error-codes).

## Logging and output

- Every user-facing message comes from one message registry. This covers CLI output, error and diagnostic messages, notices, help text, and language server and report text. No literal message at a call site. Three exemptions apply, per the design on `slopvac-8l4.21.6`: clap's built-in text (usage errors and section headings) stays with clap, prose on generated reference pages is exempt because the prose gate covers it, and `frontmatter-spans` takes no registry dependency because its callers wrap its errors. Each stable error code maps to a message template with named parameters. The docs generator reads the same registry. A CI check rejects bypassing literals. Why: one source keeps wording, codes and generated docs in step. Target: after `slopvac-8l4.21.6`. Source: [policies, error codes](contract/policies.md#35-error-codes).
- Apart from the three exemptions above, the message registry follows these rules, so a later move to Fluent needs no rewrite. Every message code is a valid Fluent message id. Parameters are named, never positional. All rendering goes through one function in `slopvac-messages`, never through per-variant `Display` bodies. Parameter types are plain values (strings, numbers, redacted paths), and counts are marked for later plural selection. Keep text out of code where possible, and add a test that exports the whole catalog as `.ftl` to prove compatibility. Add no Fluent dependency now. Target: after `slopvac-8l4.21.6`.
- Libraries stay silent. No `println!`, `eprintln!` or `dbg!` in a library crate. They emit `tracing` events and spans and return values. Why: the binary owns the streams. Target for the lint that enforces it: after `slopvac-8l4.21.3`. Source: skill `lang-rust-libraries`.
- The binary installs one stderr subscriber, and `SLOPVAC_LOG` turns it on. Logging is off when the variable is unset, and no log file exists. Sources: [`logging.rs`](../crates/slopvac/src/logging.rs), [policies, logging](contract/policies.md#17-logging-and-redaction).
- The language server writes only protocol frames to stdout. Why: stdout is the transport. Source: [plan, Language and runtime](design/slopvac-3.md#language-and-runtime).
- Logs, panics and error chains never carry document text, matched spans, front matter values, terms, credentials or `Authorization` headers. Show paths relative to the project root, `<external>/<file name>` outside it, and `<stdin>` for stdin. Strip URL user-info before logging. Source: [policies, logging](contract/policies.md#17-logging-and-redaction).
- Escape output through `slopvac-sanitize` only. Do not write another escaper for terminal, XML, GitHub or HTML output. Why: escaping defects are injection defects. Target: after `slopvac-8l4.2.32`, because `slopvac-report` still has its own escapers. `slopvac-sanitize` already offers terminal, HTML and workflow escaping, but it has no XML-specific coverage (the XML 1.0 character rule) for the report formats. Sources: [threat model](contract/threat-model.md) (O1 to O4), [`slopvac-sanitize`](../crates/slopvac-sanitize/src/lib.rs).

## Panics and lints

- No `unwrap()` in library code. Target for the enforcing lint: after `slopvac-8l4.21.3`. Source: skill `lang-rust-quality`.
- Use `expect` only for an invariant from local construction or bundled data (a regex literal, an embedded table). Mark the site `#[expect(clippy::expect_used, reason = "...")]`. Do not call `expect` on input from a file, the network, a plugin or the user. Return an error. Target: after `slopvac-8l4.21.3`.
- Every `#[allow]` and `#[expect]` has a `reason`. Use `#[expect(..., reason = "...")]` for a local suppression, so a stale one fails. A workspace-level allow carries `reason = "..."`. A crate-level temporary allow names the burn-down bead in its reason. Target: after `slopvac-8l4.21.3`.
- Use a module-level `#![allow(dead_code, unreachable_pub, missing_docs, reason = "...")]` only in test-support modules shared across more than one test binary (for example `tests/common/mod.rs`). Give each a reason. Never use one in production code, and never mark one temporary. Target: after `slopvac-8l4.21.3`.
- Never discard a `#[must_use]` result with `.ok()`. Handle or propagate it. Where ignoring is right, write `let _ = ...` with a comment, or `#[expect(clippy::let_underscore_must_use, reason = "...")]` that names why failure is harmless. Use `map_err` in a way that keeps the source error. Narrow numbers with `try_from` and return an error, never with silent saturation. Use saturating helpers only for lossless widening. Writes into a `String` cannot fail, so route them through a small per-crate infallible helper with one scoped `#[expect(clippy::let_underscore_must_use, reason = "...")]`, never a file-wide or crate-wide expect. Target: after `slopvac-8l4.21.3`. Source: skill `lang-rust-quality`.
- Set `[workspace.lints.clippy]` to `pedantic = warn`. CI denies warnings through `-D warnings`. Add these restriction and nursery lints at least: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `allow_attributes_without_reason`, `undocumented_unsafe_blocks` and `clone_on_ref_ptr`. Add `print_stdout` and `print_stderr` for library crates, and `indexing_slicing` and `string_slice` for parser crates. Why: a reviewer cannot check by eye what a lint can check on every build. Target: after `slopvac-8l4.21.3`. Source: skill `lang-rust-quality`.
- `unsafe` is forbidden. The workspace sets `unsafe_code = "forbid"`. The one exception is `slopvac-pcre2`, which wraps the PCRE2 C library: it sets `unsafe_code = "deny"` and only `src/ffi.rs` opts in. See the [approved departures](#approved-departures). Sources: [`Cargo.toml`](../Cargo.toml), [`slopvac-pcre2/Cargo.toml`](../crates/slopvac-pcre2/Cargo.toml).
- Code builds clean under the workspace lints. `cargo clippy --workspace --all-targets -- -D warnings` passes. Source: [CONTRIBUTING, Rust workspace](../CONTRIBUTING.md#rust-workspace).

## Workspace and manifests

- Declare every dependency version once under `[workspace.dependencies]` and use `dep.workspace = true` in the crate. This includes dev-dependencies, patch pins and feature sets. Target: after `slopvac-8l4.21.5`, because 15 dependency declarations are still inline in member manifests. Source: skill `lang-rust-packaging`.
- Every crate except `slopvac-pcre2` sets `[lints] workspace = true`. Every crate inherits `version`, `edition`, `rust-version` and `license` from `[workspace.package]`. Source: [`Cargo.toml`](../Cargo.toml).
- Keep the toolchain pin and the `rust-version` floor. `rust-toolchain.toml` pins the dev toolchain. `rust-version` in `Cargo.toml` trails stable by two minor releases. Raise it only with a recorded reason. Source: [`Cargo.toml`](../Cargo.toml).
- Pin a dependency only with a comment that gives the reason. Source: [`Cargo.toml`](../Cargo.toml) (`pcre2-sys`, `saphyr-parser`).
- A new crate needs a `Cargo.lock` entry in `extra-files` of `release-please-config.json`. Run `just sync-release-please`. Source: [CONTRIBUTING, Rust workspace](../CONTRIBUTING.md#rust-workspace).

## API surface

- `lib.rs` is a thin facade. Declare modules private, mark items `pub(crate)` by default, and re-export a curated list with `pub use`. Why: every `pub` item is a promise. Target: after `slopvac-8l4.12.33` (CLI), `slopvac-8l4.3.50` (engine) and `slopvac-8l4.2.32` (contract and plugins), because `slopvac`, `slopvac-config`, `slopvac-agents`, `slopvac-plugins` and `slopvac-update` still expose many public modules. Source: skill `lang-rust-packaging`.
- No glob re-exports (`pub use x::*`). Target: after `slopvac-8l4.3.50`, because `slopvac-rules` still re-exports `slopvac-model` with a glob. Source: skill `lang-rust-packaging`.
- Public contract types are `#[non_exhaustive]` and have constructors. This covers the types in `slopvac-model` and the plugin protocol messages. Why: adding a field or variant must not break callers. Target: after `slopvac-8l4.3.50` (model) and `slopvac-8l4.2.32` (protocol).
- Every public item has docs, and every `lib.rs` opens with a one-sentence purpose. The workspace denies broken intra-doc links and warns on missing docs. Sources: [`Cargo.toml`](../Cargo.toml), [slopvac-3 plan, Generated documentation section](design/slopvac-3.md#generated-documentation).

## Dependencies and crate layout

- Dependencies point downward. A leaf crate depends on no crate above it. The `slopvac` binary wires the libraries together. Dev-dependency cycles are also not allowed. Target for the existing `slopvac-migrate` to `slopvac-agents` dev cycle: after `slopvac-8l4.14.26`. Source: skill `lang-rust-packaging`.
- A library may expose a clap `command()` and `run()` for the CLI to wire. The CLI relies on that pattern. Sources: [`cli.rs`](../crates/slopvac/src/cli.rs), bead `slopvac-8l4.12.13`.
- Build command trees with the clap builder API (`Command`, `Arg`) only. Why: one style, trees built by each library's `command()`, and no unused proc-macro build cost. Target: after `slopvac-8l4.21.3`, which drops the `derive` feature from `clap` in [`Cargo.toml`](../Cargo.toml).
- No async runtime. Use rayon for CPU work, and threads with crossbeam-channel for waiting work. `tokio` and `async-std` are banned in `deny.toml`. Sources: [architecture record, decision 3](design/0001-slopvac-3-architecture.md#3-no-async-runtime), [`deny.toml`](../deny.toml).
- One cohesive theme per crate, named for the theme. Never name a crate or module `util`, `common`, `misc`, `helpers` or `shared`. Open the `lib.rs` docs with a one-sentence purpose. Source: skill `lang-rust-packaging`.
- A helper with one user stays in that crate. Create a shared crate only when at least two dependants share its theme. Source: skill `quality-code-policy`.
- A shared crate holds no slopvac domain types, depends only on `std` and vetted crates, and never depends upward. Source: skill `lang-rust-packaging`.

### Where shared helpers live

| Concern | Home |
|---|---|
| Filesystem and path primitives: normalize, display under the root, redact (policies section 1.7), cache and config directories (XDG rules, UTF-8 only; see approved departures), atomic write with fsync, capped read | `slopvac-fs`. Target: after `slopvac-8l4.21.2`. Until then, do not add another copy. |
| Regex escaping | `slopvac-text`. Target: after `slopvac-8l4.3.50`, because `slopvac-rules` and `slopvac-context` each keep a local escaper. |
| Output escaping | `slopvac-sanitize`. Target: after `slopvac-8l4.2.32`, see Logging and output. |
| Line and column | `frontmatter_spans::LineIndex`. `frontmatter-spans` is an external crates.io dependency, maintained at `srobroek/frontmatter-spans`. Index lines and columns only through its public `LineIndex` API and add no slopvac-specific code to it. Target: after `slopvac-8l4.2.32`, because `slopvac-config` copies it and `slopvac-terms` has its own. |
| Content digests | One digest type in `slopvac-plugins`. Target: after `slopvac-8l4.2.32`, because the crate has two digest types. |

Why: the audit found five copies of root-relative display, four path normalizers, eight atomic writers and nine capped readers, each with different edge cases. Sources: [policies, logging](contract/policies.md#17-logging-and-redaction), [policies, data written to disk](contract/policies.md#18-data-written-to-disk), [positions contract](contract/positions.md), skill `quality-code-policy`.

## Configuration and paths

- Resolve config through the layer stack in `slopvac-config`. Every value keeps its file, line and column. Sources: [architecture record, decision 6](design/0001-slopvac-3-architecture.md#6-hand-rolled-config-layer-stack-with-provenance), [decision 20](design/0001-slopvac-3-architecture.md#20-config-precedence-nearest-config-and-front-matter).
- Write only to the per-user cache directory, the project's `.slopvac/` directory and paths the user names. Source: [policies, data written to disk](contract/policies.md#18-data-written-to-disk).
- Use no prompt in any code path. Agents and CI cannot answer one. Source: [architecture record, decision 9](design/0001-slopvac-3-architecture.md#9-oci-artifacts-minisign-and-mise-style-trust).
- Write no native Windows code in 3.0. Do not add `cfg(windows)` branches or Windows dependencies. Windows users run the Linux build under WSL. Target for the existing Windows code: removal after `slopvac-8l4.21.5`. Source: [architecture record, decision 26](design/0001-slopvac-3-architecture.md#26-no-native-windows-in-30).

## User-facing text

- Messages, help and docs never name internal IDs, bead IDs, PR numbers or development history. Why: users cannot act on them, and they go stale. Source: the prose rules from `slopvac prime write`. This page is exempt because it is a contributor document.
- Message text may change in any release. Codes, rule IDs and exit codes are the interface. Source: [policies, error codes](contract/policies.md#35-error-codes).

## Testing

Detail and tool choice: [`testing-rust.md`](testing-rust.md).

- Run tests with `cargo nextest run` and run doctests with `cargo test --doc`. nextest skips doctests. Source: [testing, Choose the tool](testing-rust.md#choose-the-tool).
- Write one behavior per test, through the public interface. Work out expected values apart from the code under test. Sources: [testing, Rules](testing-rust.md#rules), skill `quality-testing`.
- Do not mirror the implementation. A refactor leaves tests unchanged. Delete the tests of removed behavior. Do not write a test that only checks code is gone. Source: skill `quality-testing`.
- Parametrize cases that differ only in data with rstest and `#[case::name]`. Source: [testing, Rules](testing-rust.md#rules).
- Use proptest for an invariant over any input (parsers, config merge, span arithmetic). Commit the `.proptest-regressions` files. Source: [testing, Property tests](testing-rust.md#property-tests).
- Review insta snapshots and never auto-accept them. CI fails on a missing or changed snapshot. Source: [testing, Snapshots](testing-rust.md#snapshots).
- Test the CLI with snapbox against the built binary. Target: after `slopvac-8l4.14.26`, because some tests still use `assert_cmd`. Source: [testing, Choose the tool](testing-rust.md#choose-the-tool).
- No `#[ignore]` without a CI job that runs ignored tests. Why: an ignored test that never runs proves nothing. Target: after `slopvac-8l4.14.26`, which adds the job.
- Fake only at boundaries the project owns (network, process, clock, filesystem). Do not mock a type you do not own. Source: skill `quality-testing`.
- Never weaken an assertion or edit an expected value just to pass. Source: [testing, Rules](testing-rust.md#rules).

## Supply chain and documentation

- `cargo deny check` passes. Licences are permissive only. Never add `CC-BY-SA-4.0` to the global allow list. Scope it to the crate that embeds the model. Sources: [`deny.toml`](../deny.toml), [CONTRIBUTING, Rust checks](../CONTRIBUTING.md#rust-checks).
- `cargo shear`, `cargo sort` and `typos` pass. No unused dependency, sorted manifests. Source: [CONTRIBUTING, Rust checks](../CONTRIBUTING.md#rust-checks).
- Before you push, run each Rust check:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo nextest run --workspace`
  - `cargo test --doc --workspace`
  - `prek run --all-files`, which covers the manifest and spelling hooks but not clippy, nextest or doctests.

  Source: [CONTRIBUTING, Rust workspace](../CONTRIBUTING.md#rust-workspace) and [Rust checks](../CONTRIBUTING.md#rust-checks).
- Lint documentation with the prose gate: `uvx --from ./packages/slopvac-lint slopvac <files>`. Exit 2 is incomplete, not clean. Source: [CONTRIBUTING, Prose checks](../CONTRIBUTING.md#prose-checks).
- Verify each documentation claim against the code. Source: [`AGENTS.md`](../AGENTS.md).

## Approved departures

Each departure from a standard below has a recorded reason. Where a reason is not recorded in code yet, a bead tracks it.

| Departure | Standard it departs from | Reason | Source |
|---|---|---|---|
| Typed `CliError` instead of `anyhow` in the binary | [Architecture record, decision 10](design/0001-slopvac-3-architecture.md#10-error-and-diagnostic-rendering-thiserror-anyhow-annotate-snippets) | Error codes are a contract, and `anyhow` has no typed code. A later change amends the architecture record. | [`errors.rs`](../crates/slopvac/src/errors.rs), [policies, error codes](contract/policies.md#35-error-codes) |
| `std::fs::File` locking instead of `fs4` | [Policies, data written to disk](contract/policies.md#18-data-written-to-disk), [plan library table](design/slopvac-3.md#language-and-runtime) | The standard library provides file locks, so `fs4` adds a dependency for no gain. A later change amends the plan and the threat model. | [Threat model](contract/threat-model.md) (E9) |
| No `axoupdater` | [Plan library table](design/slopvac-3.md#language-and-runtime) | It downloads and runs the installer script, so no point exists between download and replace to check the minisign signature. | [`slopvac-update/src/lib.rs`](../crates/slopvac-update/src/lib.rs) |
| Hand-rolled update check instead of `update-informer` | [Plan library table](design/slopvac-3.md#language-and-runtime) | The check follows the install method, the update policy and the 24-hour notice cache. The code does not yet record this reason. | [`notice.rs`](../crates/slopvac-update/src/notice.rs), [policies, inventory](contract/policies.md#12-inventory-of-network-features) |
| `cap-std` handle-relative writes in `slopvac-agents` | `slopvac-fs` atomic write | Writes go through a directory handle opened once, which closes the time-of-check to time-of-use gap. | [`fsx.rs`](../crates/slopvac-agents/src/fsx.rs) |
| Hand-rolled config layer stack with provenance | Use `schematic` | `schematic` has no per-list-element provenance, no spans for YAML errors, reports the first error only and overflows the stack on a cyclic `extends`. | [Architecture record, decision 6](design/0001-slopvac-3-architecture.md#6-hand-rolled-config-layer-stack-with-provenance) |
| Hand-rolled OCI client on `ureq` | Use `oci-client` | `oci-client` needs tokio and adds 180 dependencies and 5.4 MB, against 42 dependencies and 1.67 MB for the client on `ureq`. No async runtime is allowed. | [Architecture record, decision 3](design/0001-slopvac-3-architecture.md#3-no-async-runtime) |
| `cargo shear` for unused dependencies | `cargo machete` | The plan names `cargo shear` in its hygiene list. No document or commit records a comparison with `cargo machete`. | [Plan, Quality](design/slopvac-3.md#quality), [CONTRIBUTING, Rust checks](../CONTRIBUTING.md#rust-checks) |
| Span-keeping JSON reader in `slopvac-diagrams` | `serde_json` | `serde_json` gives no byte offsets, and the extractors need them to report spans. | [`json.rs`](../crates/slopvac-diagrams/src/json.rs) |
| `slopvac-pcre2` sets its own `[lints]` with `unsafe_code = "deny"` and allows `unsafe` in `src/ffi.rs` | Workspace `unsafe_code = "forbid"` and `lints.workspace = true` | The crate wraps the PCRE2 C library through FFI. `forbid` cannot be overridden, so the crate cannot inherit the workspace table. It repeats the other workspace lints, and only `src/ffi.rs` opts in with `#![allow(unsafe_code)]`. Residual risk: the table can drift from the root. Revisit if the FFI layer is replaced. | [`slopvac-pcre2/Cargo.toml`](../crates/slopvac-pcre2/Cargo.toml), [`ffi.rs`](../crates/slopvac-pcre2/src/ffi.rs) |
| `pcre2-sys` pinned to `=0.2.10` | Floating dependency version | Version 0.2.10 vendors PCRE2 10.46, and the rules need 10.43 or later. | [`Cargo.toml`](../Cargo.toml) |
| `jsonschema` with `default-features = false` | Default features | The default features pull in a transitive async runtime (tokio), and `deny.toml` bans `tokio`, `async-std` and OpenSSL. | [`Cargo.toml`](../Cargo.toml), [`deny.toml`](../deny.toml) |
| `slopvac-fs` path-bearing errors scrub the wrapped `io` source to its kind and raw OS code | "`map_err` keeps the source error" | Policies section 1.7 forbids paths outside the root in error chains, and an `io` source can carry one. The path stays reachable through `Error::path()`. Scope: `slopvac-fs` only. Revisit if policies section 1.7 changes or the crate stops wrapping `io` errors. | [`slopvac-fs/src/error.rs`](../crates/slopvac-fs/src/error.rs), [policies, logging](contract/policies.md#17-logging-and-redaction) |
| `slopvac-fs` per-user directory functions accept UTF-8 only and fail on a non-UTF-8 variable | Platform path handling (`OsString`), and the `etcetera` behaviour | Directory paths come from configuration and output that must be UTF-8, so a non-UTF-8 value has nowhere to go. The functions return a typed error that names the variable and never its value. They apply the XDG rules to an injected lookup and are not an `etcetera` equivalent. Residual risk: a user with a non-UTF-8 `HOME` or `XDG_*` value cannot run. Revisit if configuration or output stops requiring UTF-8. | [`slopvac-fs/src/dirs.rs`](../crates/slopvac-fs/src/dirs.rs) |
| `slopvac-plugins` `BadSignature` stays a public unit variant with no payload | "`map_err` keeps the source error" | Callers must not learn why verification failed. The code logs the source at debug level through `tracing`, without key material. Scope: the three sites in `sign.rs` and the one in `revoke.rs`. Revisit if the plugin protocol needs a failure reason or the error enums change under `slopvac-8l4.2.32`. | [`sign.rs`](../crates/slopvac-plugins/src/sign.rs), [`revoke.rs`](../crates/slopvac-plugins/src/revoke.rs) |
| `#[derive(Catalog)]` supplies `Display`, `Error` and `From` instead of `thiserror` | Libraries define errors with `thiserror` | A delegating variant must forward `source()` to the wrapped error's own source while all rendering goes through one function in `slopvac-messages`. The `source()` that `thiserror` generates cannot do both. Conditions that still hold: the types are `std::error::Error` with a working `source()` chain, and `#[source]` and `#[from]` compose with `?`. Revisit when the Fluent move happens. | [Logging and output](#logging-and-output) |
| Plain-level `SLOPVAC_LOG` text subscriber on stderr, no OpenTelemetry export | Telemetry over OTLP | Policy states that slopvac has no telemetry. It permits only the listed update, registry, reporting and provider traffic, so no trace exporter may send data. Logs go to stderr only. | [Policies, principles](contract/policies.md#11-principles), [policies, inventory](contract/policies.md#12-inventory-of-network-features) |

## How to request a departure

1. Open the pull request that needs the departure. State the rule and the files it affects.
2. In the pull request description, record the reason, the alternatives you considered, the residual risk and a revisit trigger (an event or date at which to reassess).
3. Add a row to [Approved departures](#approved-departures) in the same pull request, with the same four items and a source.
4. A reviewer rejects a departure that is not recorded in both places.

## Review checklist

- [ ] Library errors use a `thiserror` enum per module and a `Result` alias (target).
- [ ] The binary reports errors through `CliError`, with a code from the registry (target).
- [ ] No code is reused or changes meaning, and a retired code stays in the docs marked retired.
- [ ] Exit codes are only 0, 1 and 2.
- [ ] Every user-facing message comes from the message registry, except clap's built-in text, generated reference prose and `frontmatter-spans`, and the registry follows the Fluent-compatible rules (target).
- [ ] No library code prints. Events go through `tracing`.
- [ ] Logging is off unless `SLOPVAC_LOG` is set, and no log file exists.
- [ ] The language server writes only protocol frames to stdout.
- [ ] No log, panic or error text carries document text, secrets or absolute paths outside the root.
- [ ] Output escaping goes through `slopvac-sanitize` (target).
- [ ] No `unwrap()` in library code. Each `expect` guards a local invariant and has `#[expect(..., reason)]` (target).
- [ ] Every `#[allow]` and `#[expect]` has a `reason`, and the workspace clippy lint set is in place (target).
- [ ] No `unsafe` outside `src/ffi.rs` in `slopvac-pcre2`.
- [ ] A module-level `#![allow(...)]` appears only in shared test-support modules and has a `reason` (target).
- [ ] No `#[must_use]` result is discarded with `.ok()`, `map_err` keeps the source error, and narrowing conversions use `try_from` (target).
- [ ] Dependencies are declared in `[workspace.dependencies]` (target).
- [ ] The crate sets `lints.workspace = true` (except `slopvac-pcre2`) and inherits package fields from `[workspace.package]`.
- [ ] The change keeps the toolchain pin and the `rust-version` floor.
- [ ] Each dependency pin has a comment that gives the reason.
- [ ] `lib.rs` is a thin facade with private modules, `pub(crate)` by default and no glob re-exports (target).
- [ ] New public contract types are `#[non_exhaustive]` with constructors (target).
- [ ] Every public item has docs, and `lib.rs` opens with a one-sentence purpose.
- [ ] Dependencies point downward. The CLI wires libraries. No async runtime.
- [ ] The CLI uses the clap builder API only (target).
- [ ] A new crate has one theme, a theme name and a one-sentence purpose. A shared crate has at least two dependants and no domain types.
- [ ] A new crate has a `release-please-config.json` entry (`just sync-release-please`).
- [ ] Filesystem, path, redaction and directory helpers come from `slopvac-fs` (target).
- [ ] Regex escaping, line and column, and digests use their single home (target).
- [ ] Config values keep their file, line and column.
- [ ] No prompt, no new native Windows code, no write outside the allowed directories.
- [ ] User-facing text has no bead IDs, PR numbers or development history.
- [ ] Each test checks one behavior through the public interface and does not mirror the code.
- [ ] Cases that differ only in data are parametrized.
- [ ] Doubles stand in only at boundaries the project owns.
- [ ] Property tests have committed regressions, and snapshots were reviewed by hand.
- [ ] No assertion or expected value was weakened to pass.
- [ ] No `#[ignore]` without a CI job that runs it (target).
- [ ] The Rust checks listed under Supply chain and documentation all pass.
- [ ] `cargo deny`, `cargo shear`, `cargo sort` and `typos` pass.
- [ ] Documentation passes the prose gate and its claims match the code.
- [ ] Every departure is recorded in the pull request and in this file.
