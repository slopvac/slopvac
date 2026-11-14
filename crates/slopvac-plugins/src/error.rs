//! Plugin host errors and their stable `slopvac_code` strings.

use thiserror::Error;

/// A JSON-RPC error object sent by a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// Numeric JSON-RPC code.
    pub code: i64,
    /// Human-readable message. Not an interface.
    pub message: String,
    /// Stable kebab-case code from `data.slopvac_code`.
    pub slopvac_code: String,
    /// The `data` object, or null.
    pub data: serde_json::Value,
}

/// Everything that can go wrong when a host talks to a plugin.
///
/// Each variant maps to a `slopvac_code` from plugin-protocol.md section 10.
/// Any plugin failure makes the affected run exit 2.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PluginError {
    /// The content hash has no recorded trust.
    #[error("plugin {0} is not trusted; run `slopvac plugins trust` for it")]
    Untrusted(String),
    /// The installed bytes differ from the hash that trust recorded.
    #[error("plugin {0} changed after it was trusted")]
    HashMismatch(String),
    /// A timer in section 6.3 expired.
    #[error("plugin {plugin} timed out: {what}")]
    Timeout {
        /// Plugin name.
        plugin: String,
        /// Which timer expired.
        what: String,
    },
    /// The process ended with requests pending.
    #[error("plugin {plugin} exited ({status}){stderr}")]
    Exited {
        /// Plugin name.
        plugin: String,
        /// Exit status description.
        status: String,
        /// Sanitised stderr tail, with a leading newline, or empty.
        stderr: String,
    },
    /// A framing error, a limit breach or a duplicate key.
    #[error("plugin protocol violation: {0}")]
    ProtocolViolation(String),
    /// A result does not match its schema or breaks a rule such as span bounds.
    #[error("plugin output invalid: {0}")]
    OutputInvalid(String),
    /// The program did not start.
    #[error("plugin failed to start: {0}")]
    SpawnFailed(String),
    /// The reply disagrees with the hashed manifest.
    #[error("plugin declaration mismatch: {0}")]
    DeclarationMismatch(String),
    /// The request counter reached 2^53-1.
    #[error("request ids exhausted")]
    RequestIdExhausted,
    /// No common protocol version.
    #[error("no common protocol version: host speaks {host}, plugin speaks {plugin}")]
    ProtocolVersionUnsupported {
        /// Host range, for example `1..=1`.
        host: String,
        /// Plugin range as reported, or the picked version.
        plugin: String,
    },
    /// The host lacks a feature the plugin requires.
    #[error("plugin requires feature `{0}`, which the host does not offer")]
    RequiredFeatureMissing(String),
    /// The plugin needs the network and offline mode is on.
    #[error("offline: {0} needs the network")]
    Offline(String),
    /// The plugin answered a request with an error object.
    #[error("plugin error {}: {} ({})", .0.code, .0.message, .0.slopvac_code)]
    Rpc(RpcError),
    /// The host cancelled the request and discarded the answer.
    #[error("request cancelled")]
    Cancelled,
    /// The hook or document does not fit a message and cannot be split.
    #[error("message too large: {0}")]
    MessageTooLarge(String),
    /// The caller used the plugin wrongly, for example an undeclared hook.
    #[error("invalid use of plugin: {0}")]
    Usage(String),
}

impl PluginError {
    /// The stable code for diagnostics and reports.
    #[must_use]
    pub fn slopvac_code(&self) -> &str {
        match self {
            Self::Untrusted(_) => "plugin-untrusted",
            Self::HashMismatch(_) => "plugin-hash-mismatch",
            Self::Timeout { .. } => "plugin-timeout",
            Self::Exited { .. } => "plugin-exited",
            Self::ProtocolViolation(_) => "plugin-protocol-violation",
            Self::OutputInvalid(_) => "plugin-output-invalid",
            Self::SpawnFailed(_) => "plugin-spawn-failed",
            Self::DeclarationMismatch(_) => "plugin-declaration-mismatch",
            Self::RequestIdExhausted => "request-id-exhausted",
            Self::ProtocolVersionUnsupported { .. } => "protocol-version-unsupported",
            Self::RequiredFeatureMissing(_) => "required-feature-missing",
            Self::Offline(_) => "offline",
            Self::Rpc(e) => &e.slopvac_code,
            Self::Cancelled => "request-cancelled",
            Self::MessageTooLarge(_) => "message-too-large",
            Self::Usage(_) => "plugin-usage",
        }
    }
}
