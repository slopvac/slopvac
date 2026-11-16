# Engineering standards

This page lists the requirements every change to the Rust code in `crates/` must meet. Reviewers, human and agent, check a change against it.

- A rule marked "target: after ..." is not met by the whole code base yet. New code follows it. Existing code moves when its owner bead lands.
- A rule without a marker holds now.
- To deviate from a rule, follow [How to request a departure](#how-to-request-a-departure).

Each rule gives a source. A source named as a skill (for example "lang-rust-packaging") is a coding-agent skill, not a repository file. Repository paths are relative to the repository root.

## Errors

- Libraries define one `thiserror` enum per module and a `Result` alias. Derive `Error`, do not write `Display` by hand, and size each enum to its operation. Exception: a `Display` that must sanitize text. Why: callers match on variants, and the derive keeps messages and sources in sync. Sources: `docs/design/0001-slopvac-3-architecture.md` decision 10; lang-rust-libraries, Errors.
- The binary reports failures through the typed `CliError`. Each error carries a stable kebab-case code. Text output renders `error[<code>]: <message>`. Why: codes are part of the interface and message text is not. Sources: `crates/slopvac/src/errors.rs` and `docs/contract/policies.md` section 3.5.
- Take every code from one registry. No free-string code at a call site. Target: after the error-code registry bead.
- Never reuse a code and never change its meaning. A retired code stays in the generated docs. Source: `docs/contract/policies.md` section 3.5.
- Use only exit codes 0 (pass), 1 (a gate failed) and 2 (run incomplete). Treat exit 2 as incomplete. Source: `docs/contract/policies.md` section 3.5.

## Logging and output

- Every user-facing message comes from one message registry. This covers CLI output, error and diagnostic messages, notices, help text, and language server and report text. No literal message at a call site. Each stable error code maps to a message template with named parameters. The docs generator reads the same registry. A CI check rejects bypassing literals. Why: one source keeps wording, codes and generated docs in step. Source: `docs/contract/policies.md` section 3.5. Target: after the message-registry design bead.
- Libraries stay silent. No `println!`, `eprintln!` or `dbg!` in a library crate. They emit `tracing` events and spans and return values. Why: the binary owns the streams. Source: lang-rust-libraries, Logging.
- The binary installs one stderr subscriber, enabled by `SLOPVAC_LOG`. Logging is off when the variable is unset. No log file. Sources: `crates/slopvac/src/logging.rs` and `docs/contract/policies.md` section 1.7.
- The language server writes only protocol frames to stdout. Why: stdout is the transport. Source: `docs/design/slopvac-3.md`, Language and runtime (`lsp-server`).
- Logs, panics and error chains never carry document text, matched spans, front matter values, terms, credentials or `Authorization` headers. Show paths relative to the project root, `<external>/<file name>` outside it, and `<stdin>` for stdin. Strip URL user-info before logging. Source: `docs/contract/policies.md` section 1.7.
- Escape output only through `slopvac-sanitize`. Do not write another escaper for terminal, XML, GitHub or HTML output. Why: escaping defects are injection defects. Sources: `docs/contract/threat-model.md` (O1 to O4). Also `crates/slopvac-sanitize/src/lib.rs`.

## Panics and lints

- No `unwrap()` in library code. Source: lang-rust-quality, Lint.
- Use `expect` only for an invariant from local construction or bundled data (a regex literal, an embedded table). Mark the site `#[expect(clippy::expect_used, reason = "...")]`. Do not call `expect` on input from a file, the network, a plugin or the user. Return an error. Target: after the clippy-gates bead.
- Every `#[allow]` and `#[expect]` has a `reason`. Use `#[expect(..., reason = "...")]` for a local suppression, so a stale one fails. A workspace-level allow carries `reason = "..."`. A crate-level temporary allow names the burn-down bead in its reason. Target: after the lint-config bead.
- Set `[workspace.lints.clippy]` to `pedantic = warn`. CI denies warnings through `-D warnings`. Add these restriction and nursery lints at least: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `allow_attributes_without_reason`, `undocumented_unsafe_blocks` and `clone_on_ref_ptr`. Add `print_stdout` and `print_stderr` for library crates, and `indexing_slicing` and `string_slice` for parser crates. Why: a reviewer cannot check by eye what a lint can check on every build. Source: lang-rust-quality, Lint. Target: after the lint-config bead.
- `unsafe` is forbidden. The workspace sets `unsafe_code = "forbid"`. Source: `Cargo.toml`, `[workspace.lints.rust]`.
- Code builds clean under the workspace lints. `cargo clippy --workspace --all-targets -- -D warnings` passes. Source: `CONTRIBUTING.md`, Rust workspace.

## Workspace and manifests

- Declare every dependency version once under `[workspace.dependencies]` and use `dep.workspace = true` in the crate. This includes dev-dependencies, patch pins and feature sets. Source: lang-rust-packaging, Workspace manifest.
- Every crate sets `[lints] workspace = true` and inherits `version`, `edition`, `rust-version` and `license` from `[workspace.package]`. Source: `Cargo.toml`.
- Keep the toolchain and the floor. `rust-toolchain.toml` pins the dev toolchain. `rust-version` in `Cargo.toml` trails stable by two minor releases. Raise it only with a recorded reason. Source: `Cargo.toml`, `[workspace.package]`.
- Pin a dependency only with a comment that gives the reason. Source: `Cargo.toml` (`pcre2-sys`, `saphyr-parser`).
- A new crate needs a `Cargo.lock` entry in `extra-files` of `release-please-config.json`. Run `just sync-release-please`. Source: `CONTRIBUTING.md`, Rust workspace.

## API surface

- `lib.rs` is a thin facade. Declare modules private, mark items `pub(crate)` by default, and re-export a curated list with `pub use`. Why: every `pub` item is a promise. Source: lang-rust-packaging, Crate boundaries.
- No glob re-exports (`pub use x::*`). Source: lang-rust-packaging, Crate boundaries.
- Public contract types are `#[non_exhaustive]` and have constructors. This covers the types in `slopvac-model` and the plugin protocol messages. Why: adding a field or variant must not break callers. Target: the engine and contract beads that own those crates.
- Every public item has docs, and every `lib.rs` opens with a one-sentence purpose. The workspace denies broken intra-doc links. Source: `Cargo.toml`, `[workspace.lints]`. Also `docs/design/slopvac-3.md`, Generated documentation.

## Dependencies and crate layout

- Dependencies point downward. A leaf crate depends on no crate above it. The `slopvac` binary wires the libraries together. Dev-dependency cycles are also not allowed. Source: lang-rust-packaging, Crate boundaries.
- A library may expose a clap `command()` and `run()` for the CLI to wire. The CLI relies on that pattern. Source: `crates/slopvac/src/cli.rs`; bead 8l4.12.13.
- Build command trees with the clap builder API (`Command`, `Arg`) only. Why: one style, trees built by each library's `command()`, and no unused proc-macro build cost. Target: after the clap-derive removal bead, which drops the `derive` feature from `clap` in `Cargo.toml`.
- No async runtime. Use rayon for CPU work, and threads with crossbeam-channel for waiting work. `tokio` and `async-std` are banned in `deny.toml`. Source: `docs/design/0001-slopvac-3-architecture.md` decision 3. Also `deny.toml`, `[bans]`.
- One cohesive theme per crate, named for the theme. Never name a crate or module `util`, `common`, `misc`, `helpers` or `shared`. Open the `lib.rs` docs with a one-sentence purpose. Source: lang-rust-packaging, Crate boundaries.
- A helper with one user stays in that crate. Create a shared crate only when at least two dependants share its theme. Source: quality-code-policy, DRY.
- A shared crate holds no slopvac domain types, depends only on `std` and vetted crates, and never depends upward. Source: lang-rust-packaging, Crate boundaries.

### Where shared helpers live

| Concern | Home |
|---|---|
| Filesystem and path primitives: normalize, display under the root, redact (policies section 1.7), cache and config directories through `etcetera`, atomic write with fsync, capped read | `slopvac-fs`. Target: after the slopvac-fs bead. |
| Regex escaping | `slopvac-text` |
| Output escaping | `slopvac-sanitize` |
| Line and column | `LineIndex` in `frontmatter-spans`. Config and terms use it directly. |
| Content digests | One digest type in `slopvac-plugins` |

Why: the audit found five copies of root-relative display, four path normalizers, eight atomic writers and nine capped readers, each with different edge cases. Sources: `docs/contract/policies.md` sections 1.7 and 1.8. Also `docs/contract/positions.md`; quality-code-policy, DRY.

## Configuration and paths

- Resolve config through the layer stack in `slopvac-config`. Every value keeps its file, line and column. Source: `docs/design/0001-slopvac-3-architecture.md` decisions 6 and 20.
- Write only to the per-user cache directory, the project's `.slopvac/` directory and paths the user names. Source: `docs/contract/policies.md` section 1.8.
- Use no prompt in any code path. Agents and CI cannot answer one. Source: `docs/design/0001-slopvac-3-architecture.md` decision 9.
- Write no native Windows code in 3.0. Do not add `cfg(windows)` branches or Windows dependencies. Windows users run the Linux build under WSL. Source: `docs/design/0001-slopvac-3-architecture.md` decision 26.

## User-facing text

- Messages, help and docs never name internal IDs, bead IDs, PR numbers or development history. Why: users cannot act on them, and they go stale. Source: the prose rules from `slopvac prime write`. This page is exempt because it is a contributor document.
- Message text may change in any release. Codes, rule IDs and exit codes are the interface. Source: `docs/contract/policies.md` section 3.5.

## Testing

Detail and tool choice: [`docs/testing-rust.md`](testing-rust.md).

- Run tests with `cargo nextest run` and run doctests with `cargo test --doc`. nextest skips doctests. Source: `docs/testing-rust.md`, Choose the tool.
- Write one behavior per test, through the public interface. Work out expected values apart from the code under test. Source: `docs/testing-rust.md`, Rules; quality-testing.
- Do not mirror the implementation. A refactor leaves tests unchanged. Delete the tests of removed behavior. Do not write a test that only checks code is gone. Source: quality-testing.
- Parametrize cases that differ only in data with rstest and `#[case::name]`. Source: `docs/testing-rust.md`, Rules.
- Use proptest for an invariant over any input (parsers, config merge, span arithmetic). Commit the `.proptest-regressions` files. Source: `docs/testing-rust.md`, Property tests.
- Review insta snapshots and never auto-accept them. CI fails on a missing or changed snapshot. Source: `docs/testing-rust.md`, Snapshots.
- Test the CLI with snapbox against the built binary. Source: `docs/testing-rust.md`, Choose the tool.
- No `#[ignore]` without a CI job that runs ignored tests. Why: an ignored test that never runs proves nothing. Target: after the Rust CI bead adds the job.
- Fake only at boundaries the project owns (network, process, clock, filesystem). Do not mock a type you do not own. Source: quality-testing, Doubles.
- Never weaken an assertion or edit an expected value just to pass. Source: `docs/testing-rust.md`, Rules.

## Supply chain and documentation

- `cargo deny check` passes. Licences are permissive only. Never add `CC-BY-SA-4.0` to the global allow list: scope it to the crate that embeds the model. Sources: `deny.toml`. Also `CONTRIBUTING.md`, Rust checks.
- `cargo shear`, `cargo sort` and `typos` pass. No unused dependency, sorted manifests. Source: `CONTRIBUTING.md`, Rust checks.
- Run rustfmt, clippy, nextest and doctests before you push. `prek run --all-files` runs the same checks. Source: `CONTRIBUTING.md`, Rust checks.
- Lint documentation with the prose gate: `uvx --from ./packages/slopvac-lint slopvac <files>`. Exit 2 is incomplete, not clean. Source: `CONTRIBUTING.md`, Prose checks.
- Verify each documentation claim against the code. Source: `AGENTS.md`.

## Approved departures

Each departure from a standard below has a recorded reason. Where a reason is not recorded in code yet, a bead tracks it.

| Departure | Standard it departs from | Reason | Source |
|---|---|---|---|
| Typed `CliError` instead of `anyhow` in the binary | architecture record decision 10 | Error codes are a contract, and `anyhow` has no typed code. A later change amends the architecture record. | `crates/slopvac/src/errors.rs`. Also `docs/contract/policies.md` section 3.5 |
| `std::fs::File` locking instead of `fs4` | `docs/contract/policies.md` section 1.8; plan library table | The standard library provides file locks, so `fs4` adds a dependency for no gain. A later change amends the plan and threat model. | `docs/design/slopvac-3.md`, Paths, writes, locks. Also `docs/contract/threat-model.md` E9 |
| No `axoupdater` | `docs/design/slopvac-3.md`, Updates | It downloads and runs the installer script, so no point exists between download and replace to check the minisign signature. | `crates/slopvac-update/src/lib.rs` |
| Hand-rolled update check instead of `update-informer` | `docs/design/slopvac-3.md`, Updates | The check follows the install method, the update policy and the 24-hour notice cache. The reason is not yet recorded in the code. | `crates/slopvac-update/src/notice.rs`. Also `docs/contract/policies.md` section 1.2 |
| `cap-std` handle-relative writes in `slopvac-agents` | `slopvac-fs` atomic write | Writes go through a directory handle opened once, which closes the time-of-check to time-of-use gap. | `crates/slopvac-agents/src/fsx.rs` |
| Hand-rolled config layer stack with provenance | Use `schematic` | `schematic` has no per-list-element provenance, no spans for YAML errors, reports the first error only and overflows the stack on a cyclic `extends`. | `docs/design/0001-slopvac-3-architecture.md` decision 6 |
| Hand-rolled OCI client on `ureq` | Use `oci-client` | `oci-client` needs tokio and adds 180 dependencies and 5.4 MB, against 42 dependencies and 1.67 MB for the client on `ureq`. No async runtime is allowed. | `docs/design/0001-slopvac-3-architecture.md` decision 3 |
| `cargo shear` for unused dependencies | `cargo machete` | The plan names `cargo shear` in its hygiene list. No document or commit records a comparison with `cargo machete`. | `docs/design/slopvac-3.md`, Quality. Also `CONTRIBUTING.md`, Rust checks |
| Span-keeping JSON reader in `slopvac-diagrams` | `serde_json` | `serde_json` gives no byte offsets, and the extractors need them to report spans. | `crates/slopvac-diagrams/src/json.rs` |
| `pcre2-sys` pinned to `=0.2.10` | Floating dependency version | Version 0.2.10 vendors PCRE2 10.46, and the rules need 10.43 or later. | `Cargo.toml` |
| `jsonschema` with `default-features = false` | Default features | The default features pull in `reqwest` and `tokio`, which `deny.toml` bans. | `Cargo.toml`. Also `deny.toml` |
| Plain-level `SLOPVAC_LOG` text subscriber, no OpenTelemetry | Telemetry over OTLP | The CLI is offline and sends no data. Logs go to stderr only. | `docs/contract/policies.md` section 1.7. Also `docs/design/slopvac-3.md`, Logging |

## How to request a departure

1. Open the pull request that needs the departure. State the rule and the files it affects.
2. In the pull request description, record the reason, the alternatives you considered, the residual risk and a revisit trigger (an event or date at which to reassess).
3. Add a row to [Approved departures](#approved-departures) in the same pull request, with the same four items and a source.
4. A reviewer rejects a departure that is not recorded in both places.

## Review checklist

- [ ] Library errors use a `thiserror` enum per module and a `Result` alias.
- [ ] The binary reports errors through `CliError`, with a code from the registry (target).
- [ ] Exit codes are only 0, 1 and 2.
- [ ] No library code prints. Events go through `tracing`.
- [ ] The language server writes only protocol frames to stdout.
- [ ] No log, panic or error text carries document text, secrets or absolute paths outside the root.
- [ ] Output escaping goes through `slopvac-sanitize`.
- [ ] No `unwrap()` in library code. Each `expect` guards a local invariant and has `#[expect(..., reason)]` (target).
- [ ] Every `#[allow]` and `#[expect]` has a `reason`, and the workspace clippy lint set is in place (target).
- [ ] Every user-facing message comes from the message registry (target).
- [ ] No `unsafe`.
- [ ] Dependencies are declared in `[workspace.dependencies]`, and the crate sets `lints.workspace = true`.
- [ ] `lib.rs` is a thin facade with private modules, `pub(crate)` by default and no glob re-exports.
- [ ] New public contract types are `#[non_exhaustive]` with constructors (target).
- [ ] Dependencies point downward. The CLI wires libraries. No async runtime.
- [ ] The CLI uses the clap builder API only (target).
- [ ] A new crate has one theme, a theme name and a one-sentence purpose. A shared crate has at least two dependants and no domain types.
- [ ] A new crate has a `release-please-config.json` entry (`just sync-release-please`).
- [ ] Filesystem, path, redaction and directory helpers come from `slopvac-fs` (target).
- [ ] Regex escaping, line and column, and digests use their single home.
- [ ] No prompt, no native Windows code, no write outside the allowed directories.
- [ ] User-facing text has no bead IDs, PR numbers or development history.
- [ ] Each test checks one behavior through the public interface and does not mirror the code.
- [ ] Property tests have committed regressions, and snapshots were reviewed by hand.
- [ ] No `#[ignore]` without a CI job that runs it (target).
- [ ] `cargo deny`, `cargo shear`, `cargo sort` and `typos` pass.
- [ ] Documentation passes the prose gate and its claims match the code.
- [ ] Every departure is recorded in the pull request and in this file.
