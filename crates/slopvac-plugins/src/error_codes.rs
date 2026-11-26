//! The error codes of plugin-protocol.md section 10, as the one table that both the host and the
//! document read.
//!
//! [`RPC_CODES`] maps the numeric JSON-RPC codes to their `slopvac_code`.
//! [`RpcError::code_name`](crate::RpcError::code_name) reads it, so a code the host reports has a
//! row. [`HOST_CODES`] lists the codes the host raises in diagnostics with no numeric code.
//! `protocol_error_tables` renders both as the two Markdown tables of section 10. A test
//! compares that text with the committed document, so a row without a mapping and a mapping
//! without a row both fail the build.

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
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostCode {
    /// The stable code.
    pub(crate) name: &'static str,
    /// Whether this crate's message registry defines the code. A code that another crate
    /// registers, or that only the contract defines, is `false`.
    pub(crate) registered_here: bool,
    /// The cause column of the document.
    pub(crate) cause: &'static str,
}

#[cfg(test)]
impl HostCode {
    const fn new(name: &'static str, registered_here: bool, cause: &'static str) -> Self {
        Self {
            name,
            registered_here,
            cause,
        }
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
#[cfg(test)]
pub(crate) const HOST_CODES: &[HostCode] = &[
    HostCode::new(
        "plugin-untrusted",
        true,
        "The content hash has no recorded trust. The message prints the `slopvac plugins trust` command.",
    ),
    HostCode::new(
        "trust-key-missing",
        true,
        "Trust records exist but the key they were made under is gone.",
    ),
    HostCode::new(
        "trust-key-mismatch",
        true,
        "The trust records were made under another key than the store's.",
    ),
    HostCode::new(
        "trust-key-unusable",
        true,
        "The trust key file is not private to the user or does not hold a key.",
    ),
    HostCode::new(
        "plugin-hash-mismatch",
        true,
        "The installed bytes differ from the hash that trust recorded.",
    ),
    HostCode::new(
        "plugin-not-code",
        true,
        "`slopvac plugins trust` or `untrust` named a data plugin, which runs no code and needs no trust.",
    ),
    HostCode::new("plugin-timeout", true, "A timer in section 6.3 expired."),
    HostCode::new(
        "plugin-exited",
        true,
        "The process ended with requests pending.",
    ),
    HostCode::new(
        "plugin-protocol-violation",
        true,
        "A framing error, a limit breach or a duplicate key.",
    ),
    HostCode::new(
        "plugin-output-invalid",
        true,
        "A result does not match its schema, or breaks a rule such as span bounds.",
    ),
    HostCode::new("plugin-spawn-failed", true, "The program did not start."),
    HostCode::new(
        "launcher-not-installed",
        true,
        "The running program did not install the plugin launcher, so the host cannot start a plugin or a git plugin source. Nothing was spawned.",
    ),
    HostCode::new(
        "plugin-declaration-mismatch",
        true,
        "The reply disagrees with the hashed manifest (section 4.3).",
    ),
    HostCode::new(
        "request-id-exhausted",
        true,
        "The request counter reached 2^53-1.",
    ),
    HostCode::new(
        "offline-network-needed",
        false,
        "A command that needs the network ran in offline mode. The update commands share this code.",
    ),
    HostCode::new(
        "plugin-usage",
        true,
        "The caller used the plugin wrongly, for example an undeclared hook.",
    ),
    HostCode::new(
        "binary-format-mismatch",
        false,
        "The signature and the name of a file disagree about its format (section 5.5.1).",
    ),
    HostCode::new(
        "binary-no-source",
        false,
        "A binary extractor found no embedded source in a valid container. The level depends on the name (section 5.5.4).",
    ),
    HostCode::new(
        "binary-input-changed",
        false,
        "A binary file grew between the preflight and the read (section 5.5.6).",
    ),
    HostCode::new(
        "binary-format-ambiguous",
        false,
        "Two or more enabled hooks claim a format, and the config maps it to none of them (section 5.5.1).",
    ),
    HostCode::new(
        "input-unsupported",
        false,
        "The host's existing code for a named input that it does not lint. It also covers a file the user named whose format no enabled extractor reads, such as EPUB or PDF. The document goes in `unchecked`, `data.format` names the detected format (section 5.5.1), and the run exits 2. The binary extraction contract adds no code of its own for it.",
    ),
    HostCode::new(
        "extract-unsupported",
        false,
        "An `info` notice for content that an extractor skipped inside a document it read (section 5.5.4). A plugin lists the content in the result's `unsupported` field and sends no error number. The run exits 0.",
    ),
    HostCode::new(
        "extract-output-invalid",
        false,
        "The host validator rejects the output of a first-party (built-in) extractor (section 5.5). Output from a third-party plugin gets `plugin-output-invalid` instead. The cause is a slopvac bug, and the message points to the issue tracker. It is separate from `plugin-output-invalid`, which covers plugin output, and has the same severity and exit.",
    ),
];

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

    const DOC: &str = include_str!("../../../docs/contract/plugin-protocol.md");

    fn block(name: &str) -> &'static str {
        let begin = format!("<!-- generated:{name} begin -->\n");
        let end = format!("<!-- generated:{name} end -->");
        let Some(start) = DOC.find(&begin) else {
            panic!("no `{begin}` marker");
        };
        let start = start + begin.len();
        let Some(len) = DOC[start..].find(&end) else {
            panic!("no `{end}` marker");
        };
        &DOC[start..start + len]
    }

    /// Table rows in section 10 that sit outside the generated blocks. The site generator
    /// (`scripts/docs/plugin_pages.py`) publishes every table row of section 10, so a row
    /// outside a block would ship without the drift check ever seeing it.
    fn rows_outside_blocks(doc: &str) -> Vec<String> {
        let Some(start) = doc.find("\n## 10. Error codes\n") else {
            return vec!["no section 10".to_owned()];
        };
        let section = &doc[start + 1..];
        let section = section
            .find("\n## 11.")
            .map_or(section, |end| &section[..end]);
        let mut inside = false;
        let mut stray = Vec::new();
        for line in section.lines() {
            if line.starts_with("<!-- generated:") && line.ends_with(" begin -->") {
                inside = true;
            } else if line.starts_with("<!-- generated:") && line.ends_with(" end -->") {
                inside = false;
            } else if !inside && line.starts_with('|') {
                stray.push(line.to_owned());
            }
        }
        stray
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
                row.registered_here,
                "`{}`: registered_here is wrong",
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
