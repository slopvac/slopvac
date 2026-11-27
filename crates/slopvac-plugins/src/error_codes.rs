//! The error codes of plugin-protocol.md section 10, as the one table that both the host and the
//! document read.
//!
//! [`RPC_CODES`] maps the numeric JSON-RPC codes to their `slopvac_code`.
//! [`RpcError::code_name`](crate::RpcError::code_name) reads it, so a code the host reports has a
//! row. `HOST_CODES` lists the codes the host raises in diagnostics with no numeric code.
//! `protocol_error_tables` renders both as the two Markdown tables of section 10. A test
//! compares that text with the committed document, so a row without a mapping and a mapping
//! without a row both fail the build. [`protocol_codes`] returns the same two tables as data.
//! `slopvac-schemas` writes them to `docs/reference/plugin-protocol-codes.json`, the file the
//! docs site reads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One row of the numeric JSON-RPC table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RpcCode {
    /// The numeric JSON-RPC code.
    pub(crate) number: i64,
    /// The stable `slopvac_code` the code reports as.
    pub(crate) name: &'static str,
    /// Whether the host reports this number as `name`. A number the host does not map is
    /// reported as `internal-error`, though a plugin may send it.
    pub(crate) host_maps: bool,
    /// The meaning column of the document.
    pub(crate) meaning: &'static str,
}

impl RpcCode {
    const fn new(number: i64, name: &'static str, host_maps: bool, meaning: &'static str) -> Self {
        Self {
            number,
            name,
            host_maps,
            meaning,
        }
    }
}

/// One row of the table of codes with no numeric code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostCode {
    /// The stable code.
    pub(crate) name: &'static str,
    /// The cause column of the document.
    pub(crate) cause: &'static str,
}

impl HostCode {
    const fn new(name: &'static str, cause: &'static str) -> Self {
        Self { name, cause }
    }
}

/// The numeric JSON-RPC codes, in document order.
pub(crate) const RPC_CODES: &[RpcCode] = &[
    RpcCode::new(-32700, "parse-error", true, "The body is not valid JSON."),
    RpcCode::new(
        -32600,
        "invalid-request",
        true,
        "The body is not a valid message, or it is a batch array, or the plugin is shut down.",
    ),
    RpcCode::new(
        -32601,
        "method-not-found",
        true,
        "Unknown method, or a hook the plugin did not declare.",
    ),
    RpcCode::new(
        -32602,
        "invalid-params",
        true,
        "Params do not match the schema, or name an unknown hook id.",
    ),
    RpcCode::new(-32603, "internal-error", true, "The plugin failed."),
    RpcCode::new(
        -32001,
        "protocol-version-unsupported",
        true,
        "No common version. `data.supported` holds the plugin range.",
    ),
    RpcCode::new(
        -32002,
        "required-feature-missing",
        true,
        "The host lacks a feature the plugin requires.",
    ),
    RpcCode::new(
        -32003,
        "not-initialized",
        true,
        "A request arrived before `initialize` finished.",
    ),
    RpcCode::new(
        -32004,
        "message-too-large",
        true,
        "A message exceeds `maxMessageBytes`.",
    ),
    RpcCode::new(
        -32005,
        "request-cancelled",
        true,
        "The plugin stopped a request after a cancel.",
    ),
    RpcCode::new(
        -32006,
        "document-invalid",
        true,
        "The plugin cannot process this document, for example binary content.",
    ),
    RpcCode::new(
        -32007,
        "offline",
        true,
        "A provider refused the call because offline mode is on.",
    ),
    RpcCode::new(
        -32008,
        "binary-input-declined",
        false,
        "A binary extractor declines this whole document. -32008 has no other meaning. `extract-unsupported` is a host notice with no number, and a plugin never sends it as an error. A plugin sends this number with `data.reason` `unknown-format` or `variant-unsupported` (section 5.5.4).",
    ),
    RpcCode::new(
        -32009,
        "extract-malformed",
        false,
        "The bytes break the format, or an embedded payload does not decode.",
    ),
    RpcCode::new(
        -32010,
        "extract-budget",
        false,
        "A binary extractor hit a limit.",
    ),
    RpcCode::new(
        -32011,
        "binary-encrypted",
        false,
        "The container is encrypted or protected by a password. A plugin sends it for an encrypted OpenDocument package, and the host raises it for an encrypted Office Open XML package (section 5.5.4).",
    ),
];

/// The codes the host raises with no numeric code, in document order.
pub(crate) const HOST_CODES: &[HostCode] = &[
    HostCode::new(
        "plugin-untrusted",
        "The content hash has no recorded trust. The message prints the `slopvac plugins trust` command.",
    ),
    HostCode::new(
        "trust-key-missing",
        "Trust records exist but the key they were made under is gone.",
    ),
    HostCode::new(
        "trust-key-mismatch",
        "The trust records were made under another key than the store's.",
    ),
    HostCode::new(
        "trust-key-unusable",
        "The trust key file is not private to the user or does not hold a key.",
    ),
    HostCode::new(
        "plugin-hash-mismatch",
        "The installed bytes differ from the hash that trust recorded.",
    ),
    HostCode::new(
        "plugin-not-code",
        "`slopvac plugins trust` or `untrust` named a data plugin, which runs no code and needs no trust.",
    ),
    HostCode::new("plugin-timeout", "A timer in section 6.3 expired."),
    HostCode::new("plugin-exited", "The process ended with requests pending."),
    HostCode::new(
        "plugin-protocol-violation",
        "A framing error, a limit breach or a duplicate key.",
    ),
    HostCode::new(
        "plugin-output-invalid",
        "A result does not match its schema, or breaks a rule such as span bounds.",
    ),
    HostCode::new("plugin-spawn-failed", "The program did not start."),
    HostCode::new(
        "launcher-not-installed",
        "The running program did not install the plugin launcher, so the host cannot start a plugin or a git plugin source. Nothing was spawned.",
    ),
    HostCode::new(
        "plugin-declaration-mismatch",
        "The reply disagrees with the hashed manifest (section 4.3).",
    ),
    HostCode::new(
        "request-id-exhausted",
        "The request counter reached 2^53-1.",
    ),
    HostCode::new(
        "offline-network-needed",
        "A command that needs the network ran in offline mode. The update commands share this code.",
    ),
    HostCode::new(
        "plugin-usage",
        "The caller used the plugin wrongly, for example an undeclared hook.",
    ),
    HostCode::new(
        "binary-format-mismatch",
        "The signature and the name of a file disagree about its format (section 5.5.1).",
    ),
    HostCode::new(
        "binary-no-source",
        "A binary extractor found no embedded source in a valid container. The level depends on the name (section 5.5.4).",
    ),
    HostCode::new(
        "binary-input-changed",
        "A binary file grew between the preflight and the read (section 5.5.6).",
    ),
    HostCode::new(
        "binary-format-ambiguous",
        "Two or more enabled hooks claim a format, and the config maps it to none of them (section 5.5.1).",
    ),
    HostCode::new(
        "input-unsupported",
        "The host's existing code for a named input that it does not lint. It also covers a file the user named whose format no enabled extractor reads, such as EPUB or PDF. The document goes in `unchecked`, `data.format` names the detected format (section 5.5.1), and the run exits 2. The binary extraction contract adds no code of its own for it.",
    ),
    HostCode::new(
        "extract-unsupported",
        "An `info` notice for content that an extractor skipped inside a document it read (section 5.5.4). A plugin lists the content in the result's `unsupported` field and sends no error number. The run exits 0.",
    ),
    HostCode::new(
        "extract-output-invalid",
        "The host validator rejects the output of a first-party (built-in) extractor (section 5.5). Output from a third-party plugin gets `plugin-output-invalid` instead. The cause is a slopvac bug, and the message points to the issue tracker. It is separate from `plugin-output-invalid`, which covers plugin output, and has the same severity and exit.",
    ),
];

/// Both tables of plugin-protocol.md section 10, as the one data file that the docs site
/// reads (`docs/reference/plugin-protocol-codes.json`). `slopvac-schemas` generates the file
/// and its schema from the host's code table, so the site never parses the Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProtocolCodes {
    /// The numeric JSON-RPC codes, in document order.
    pub rpc: Vec<RpcCodeRow>,
    /// The codes the host raises with no numeric code, in document order.
    pub host: Vec<HostCodeRow>,
}

/// One numeric JSON-RPC code and the `slopvac_code` it reports as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RpcCodeRow {
    /// The numeric JSON-RPC code.
    pub code: i64,
    /// The stable kebab-case `data.slopvac_code`.
    pub slopvac_code: String,
    /// What the code means.
    pub meaning: String,
}

/// One code that the host raises in its own diagnostics, with no numeric code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostCodeRow {
    /// The stable kebab-case code.
    pub slopvac_code: String,
    /// What causes the host to raise it.
    pub cause: String,
}

/// The two section 10 tables, built from the same constants the host reads.
#[must_use]
pub fn protocol_codes() -> ProtocolCodes {
    ProtocolCodes {
        rpc: RPC_CODES
            .iter()
            .map(|row| RpcCodeRow {
                code: row.number,
                slopvac_code: row.name.to_owned(),
                meaning: row.meaning.to_owned(),
            })
            .collect(),
        host: HOST_CODES
            .iter()
            .map(|row| HostCodeRow {
                slopvac_code: row.name.to_owned(),
                cause: row.cause.to_owned(),
            })
            .collect(),
    }
}

#[cfg(test)]
const RPC_HEADER: &str = "| Code | `slopvac_code` | Meaning |\n|---|---|---|\n";
#[cfg(test)]
const HOST_HEADER: &str = "| `slopvac_code` | Cause |\n|---|---|\n";

/// The two tables of plugin-protocol.md section 10: the numeric JSON-RPC codes, then the
/// host codes with no number. Rows come from the same constants the host reads, so the
/// document cannot list a code the host does not know. The drift test is the only caller.
#[cfg(test)]
fn protocol_error_tables() -> (String, String) {
    let rpc = RPC_CODES
        .iter()
        .fold(String::from(RPC_HEADER), |text, row| {
            text + &format!("| {} | `{}` | {} |\n", row.number, row.name, row.meaning)
        });
    let host = HOST_CODES
        .iter()
        .fold(String::from(HOST_HEADER), |text, row| {
            text + &format!("| `{}` | {} |\n", row.name, row.cause)
        });
    (rpc, host)
}

/// The `slopvac_code` the host reports for a numeric JSON-RPC code. Any number the table
/// does not map to the host is `internal-error`.
pub(crate) fn rpc_code_name(number: i64) -> &'static str {
    RPC_CODES
        .iter()
        .find(|row| row.host_maps && row.number == number)
        .map_or("internal-error", |row| row.name)
}
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use slopvac_messages::Catalog;

    use super::{HOST_CODES, RPC_CODES, protocol_error_tables};
    use crate::error::{PluginError, RpcError};
    use crate::messages::host::HostMessage;
    use crate::messages::message_entries;

    /// Host codes that another crate registers, or that only the contract defines.
    const NOT_REGISTERED_HERE: [&str; 8] = [
        "binary-format-mismatch",
        "binary-no-source",
        "binary-input-changed",
        "binary-format-ambiguous",
        "input-unsupported",
        "extract-unsupported",
        "extract-output-invalid",
        "offline-network-needed",
    ];

    const DOC: &str = include_str!("../../../docs/contract/plugin-protocol.md");

    /// The opening run of a fenced code block: its character and length (`CommonMark`).
    fn fence_open(line: &str) -> Option<(char, usize)> {
        let text = line.trim_start_matches(' ');
        if line.len() - text.len() > 3 {
            return None;
        }
        let ch = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let run = text.chars().take_while(|&c| c == ch).count();
        // A backtick fence cannot hold a backtick in its info string.
        (run >= 3 && !(ch == '`' && text[run..].contains('`'))).then_some((ch, run))
    }

    /// Whether `line` closes a fence opened with `ch` and `run`: a run at least as long, then
    /// nothing but white space.
    fn fence_close(line: &str, ch: char, run: usize) -> bool {
        let text = line.trim_start_matches(' ');
        let found = text.chars().take_while(|&c| c == ch).count();
        line.len() - text.len() <= 3 && found >= run && text[found..].trim().is_empty()
    }

    /// What one pass over the document finds: each generated block with its text, and the
    /// table rows of section 10 outside every block. Code fences hide everything inside them.
    /// A marker is a whole line, exactly `<!-- generated:NAME begin -->` or `... end -->`, and
    /// any other line that holds the marker prefix is an error, so no reader can take a loose
    /// marker differently.
    #[derive(Debug, Default)]
    struct Scan {
        blocks: Vec<(String, String)>,
        stray: Vec<String>,
    }

    /// The name and edge of a whole-line marker: `true` for begin.
    fn marker(line: &str) -> Option<(&str, bool)> {
        let rest = line.strip_prefix("<!-- generated:")?;
        let (name, begin) = rest
            .strip_suffix(" begin -->")
            .map(|name| (name, true))
            .or_else(|| rest.strip_suffix(" end -->").map(|name| (name, false)))?;
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        valid.then_some((name, begin))
    }

    fn scan(doc: &str) -> Result<Scan, String> {
        let mut out = Scan::default();
        let mut fence: Option<(char, usize)> = None;
        let mut open: Option<(String, String)> = None;
        let mut section = false;
        let mut heading = false;
        for line in doc.lines() {
            // A fence decides only which marker and heading lines count. Inside a block, the
            // raw text between the markers is compared, fences included.
            let mut counts = fence.is_none();
            if let Some((ch, run)) = fence {
                if fence_close(line, ch, run) {
                    fence = None;
                }
            } else if let Some(found) = fence_open(line) {
                fence = Some(found);
                counts = false;
            }
            if !counts {
                if let Some((_, text)) = open.as_mut() {
                    text.push_str(line);
                    text.push('\n');
                }
                continue;
            }
            if line.starts_with("## ") {
                section = line.starts_with("## 10. Error codes");
                heading |= section;
            }
            if line.contains("<!-- generated:") {
                match (marker(line), open.take()) {
                    (Some((name, true)), None) if section => {
                        open = Some((name.to_owned(), String::new()));
                    }
                    (Some((name, true)), None) => {
                        return Err(format!("block `{name}` is outside section 10"));
                    }
                    (Some((name, false)), Some((begun, text))) if name == begun => {
                        out.blocks.push((begun, text));
                    }
                    _ => return Err(format!("loose or unbalanced marker line: {line:?}")),
                }
            } else if let Some((_, text)) = open.as_mut() {
                text.push_str(line);
                text.push('\n');
            } else if section && line.trim_start().starts_with('|') {
                out.stray.push(line.to_owned());
            }
        }
        match open {
            Some((name, _)) => Err(format!("block `{name}` is never closed")),
            None if !heading => Err("the `## 10. Error codes` heading is missing".to_owned()),
            None => Ok(out),
        }
    }

    /// The text of the one generated block `name`. A missing or repeated block means the
    /// document drifted from the table.
    fn block_in(doc: &str, name: &str) -> Result<String, String> {
        let mut found = scan(doc)?
            .blocks
            .into_iter()
            .filter(|(block, _)| block == name);
        match (found.next(), found.next()) {
            (Some((_, text)), None) => Ok(text),
            (first, second) => Err(format!(
                "`{name}` needs exactly one block, found {}",
                usize::from(first.is_some()) + usize::from(second.is_some())
            )),
        }
    }

    fn block(name: &str) -> String {
        block_in(DOC, name).unwrap_or_default()
    }

    #[rstest]
    #[case::missing("")]
    #[case::twice(
        "<!-- generated:x begin -->\na\n<!-- generated:x end -->\n<!-- generated:x begin -->\nb\n<!-- generated:x end -->"
    )]
    fn a_missing_or_repeated_block_is_refused(#[case] doc: &str) {
        assert!(block_in(doc, "x").is_err());
    }

    /// Table rows in section 10 outside the generated blocks, or the scan error. Rows in
    /// the blocks are checked against the table. A row anywhere else is dead text that no
    /// check covers, so the gate refuses it.
    fn rows_outside_blocks(doc: &str) -> Vec<String> {
        match scan(doc) {
            Ok(found) => found.stray,
            Err(why) => vec![why],
        }
    }

    #[test]
    fn the_real_section_has_no_table_row_outside_the_blocks() {
        assert_eq!(rows_outside_blocks(DOC), Vec::<String>::new());
    }

    #[rstest]
    #[case::after_the_blocks(
        "## 10. Error codes\n<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n| -2 | `b` | extra |\n## 11. Security\n"
    )]
    #[case::before_the_blocks(
        "## 10. Error codes\n| -2 | `b` | extra |\n<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n## 11. Security\n"
    )]
    fn a_row_outside_the_blocks_is_reported(#[case] doc: &str) {
        let doc = format!("# P\n\n{doc}");
        assert_eq!(rows_outside_blocks(&doc), ["| -2 | `b` | extra |"]);
    }

    #[test]
    fn a_row_in_another_section_is_not_reported() {
        let doc =
            "# P\n\n## 9. Rules\n| x | y |\n## 10. Error codes\ntext\n## 11. Security\n| z |\n";
        assert_eq!(rows_outside_blocks(doc), Vec::<String>::new());
    }

    /// A document the gate must refuse: a row outside the blocks, or a block the site and
    /// the reader could take differently.
    fn refused(doc: &str) -> bool {
        let doc = &format!("# P\n\n{doc}");
        !rows_outside_blocks(doc).is_empty()
            || block_in(doc, "rpc-codes").is_err()
            || block_in(doc, "host-codes").is_err()
    }

    #[rstest]
    #[case::trailing_space_on_begin(
        "## 10. Error codes\n<!-- generated:rpc-codes begin --> \n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n## 11. Security\n"
    )]
    #[case::indented_row_before_the_marker(
        "## 10. Error codes\n  | -32099 | `extra` | x |\n<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n## 11. Security\n"
    )]
    #[case::prefixed_marker(
        "## 10. Error codes\n<!-- generated:other begin --><!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n## 11. Security\n"
    )]
    fn a_document_with_a_loose_marker_or_an_indented_row_is_refused(#[case] doc: &str) {
        assert!(refused(doc), "{doc}");
    }

    const GOOD_BLOCKS: &str = "<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n";

    #[rstest]
    #[case::with_the_heading("## 10. Error codes")]
    #[case::heading_renamed("## 10. Failure list")]
    fn a_row_between_unrelated_markers_is_refused(#[case] heading: &str) {
        let doc = format!(
            "{heading}\n<!-- other begin -->\n| -32020 | `unrelated` | x |\n<!-- other end -->\n{GOOD_BLOCKS}## 11. Security\n"
        );
        assert!(refused(&doc), "{doc}");
    }

    #[rstest]
    #[case::renamed("## 10. Failure list")]
    #[case::missing("")]
    fn a_missing_section_heading_is_refused(#[case] heading: &str) {
        let doc = format!("{heading}\n{GOOD_BLOCKS}| -32020 | `extra` | x |\n## 11. Security\n");
        assert!(refused(&doc), "{doc}");
    }

    #[test]
    fn a_block_outside_section_10_is_refused() {
        let doc = format!("## 9. Rules\n{GOOD_BLOCKS}## 10. Error codes\ntext\n## 11. Security\n");
        assert!(refused(&doc), "{doc}");
    }

    #[rstest]
    #[case::numeric("rpc-codes")]
    #[case::host("host-codes")]
    fn a_fenced_row_inside_a_block_is_part_of_its_text(#[case] name: &str) {
        let fenced = "```text\n| -32021 | `fenced` | In a fence. |\n```\n";
        let end = format!("<!-- generated:{name} end -->");
        let doc = DOC.replacen(&end, &format!("{fenced}{end}"), 1);
        let text = block_in(&doc, name).unwrap();
        assert!(text.contains("`fenced`"), "{text}");
        let (rpc, host) = protocol_error_tables();
        let expected = if name == "rpc-codes" { rpc } else { host };
        assert_ne!(text, expected);
    }

    #[rstest]
    #[case::fence_closed_by_a_run_with_text(
        "## 10. Error codes\n```text\n``` not a closer\n| -32022 | `inside` | Still in the fence. |\n```\n<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n## 11. Security\n"
    )]
    #[case::longer_fence_holds_a_shorter_run(
        "## 10. Error codes\n````text\n```\n| -32023 | `inside` | Still in the fence. |\n````\n<!-- generated:rpc-codes begin -->\n| -1 | `a` | m |\n<!-- generated:rpc-codes end -->\n<!-- generated:host-codes begin -->\n| `b` | c |\n<!-- generated:host-codes end -->\n## 11. Security\n"
    )]
    fn a_row_that_stays_inside_a_fence_is_not_a_row(#[case] doc: &str) {
        assert!(!refused(doc), "{doc}");
    }

    #[test]
    fn a_marker_inside_a_fence_is_not_a_marker() {
        let doc = "# P\n\n## 10. Error codes\n```md\n<!-- generated:rpc-codes begin -->\n```\n";
        assert_eq!(rows_outside_blocks(doc), Vec::<String>::new());
    }

    #[rstest]
    #[case::numeric("rpc-codes", true)]
    #[case::host("host-codes", false)]
    fn the_committed_table_matches_the_generated_one(#[case] name: &str, #[case] numeric: bool) {
        let (rpc, host) = protocol_error_tables();
        let expected = if numeric { rpc } else { host };
        assert_eq!(
            block(name),
            expected,
            "plugin-protocol.md section 10 is out of date; replace the `{name}` block with:\n{expected}"
        );
    }

    #[test]
    fn every_code_and_number_appears_once() {
        let mut numbers = HashSet::new();
        let mut names = HashSet::new();
        for row in RPC_CODES {
            assert!(numbers.insert(row.number), "number {} twice", row.number);
            assert!(names.insert(row.name), "code `{}` twice", row.name);
        }
        for row in HOST_CODES {
            assert!(names.insert(row.name), "code `{}` twice", row.name);
        }
    }

    #[test]
    fn every_host_message_code_has_a_row() {
        let known: HashSet<_> = RPC_CODES
            .iter()
            .map(|r| r.name)
            .chain(HOST_CODES.iter().map(|r| r.name))
            .collect();
        for entry in HostMessage::entries() {
            assert!(
                known.contains(entry.code),
                "`{}` has a message but no row in section 10",
                entry.code
            );
        }
    }

    #[test]
    fn a_row_says_whether_this_crate_registers_its_code() {
        let registered: HashSet<_> = message_entries().iter().map(|e| e.code).collect();
        for row in HOST_CODES {
            assert_eq!(
                registered.contains(row.name),
                !NOT_REGISTERED_HERE.contains(&row.name),
                "`{}`: the list of codes that this crate does not register is wrong",
                row.name
            );
        }
    }

    #[test]
    fn every_code_a_plugin_error_reports_has_a_row() {
        let s = || String::from("x");
        let errors = [
            PluginError::Untrusted(s()),
            PluginError::TrustKeyMissing(s()),
            PluginError::TrustKeyMismatch(s()),
            PluginError::TrustKeyUnusable(s()),
            PluginError::HashMismatch(s()),
            PluginError::Timeout {
                plugin: s(),
                what: s(),
            },
            PluginError::Exited {
                plugin: s(),
                status: s(),
                stderr: s(),
            },
            PluginError::ProtocolViolation(s()),
            PluginError::OutputInvalid(s()),
            PluginError::SpawnFailed(s()),
            PluginError::LauncherNotInstalled,
            PluginError::DeclarationMismatch(s()),
            PluginError::RequestIdExhausted,
            PluginError::ProtocolVersionUnsupported {
                host: s(),
                plugin: s(),
            },
            PluginError::RequiredFeatureMissing(s()),
            PluginError::Offline(s()),
            PluginError::Rpc(RpcError {
                code: -32603,
                message: s(),
                plugin_code: s(),
                data: serde_json::Value::Null,
            }),
            PluginError::Cancelled,
            PluginError::MessageTooLarge(s()),
            PluginError::Usage(s()),
        ];
        for error in errors {
            let code = error.slopvac_code();
            assert!(
                RPC_CODES.iter().any(|r| r.name == code)
                    || HOST_CODES.iter().any(|r| r.name == code),
                "`{code}` has no row in section 10"
            );
        }
    }

    #[test]
    fn the_code_for_a_data_plugin_has_a_row() {
        assert!(HOST_CODES.iter().any(|r| r.name == "plugin-not-code"));
    }
}
