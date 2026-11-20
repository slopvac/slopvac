//! CLI errors with stable codes (docs/contract/policies.md, 3.5).
//!
//! Text output renders `error[<code>]: <message>`. Codes are kebab-case and
//! never change meaning; message text can.

use std::fmt;

/// An error a command reports before it exits with code 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// Stable kebab-case code.
    pub code: &'static str,
    /// Human message. Not an interface.
    pub message: String,
    /// Machine-readable detail for a code that documents keys, such as a budget refusal.
    /// Empty for most errors.
    pub data: serde_json::Map<String, serde_json::Value>,
}

impl CliError {
    /// Build an error. Credentials in URLs are stripped from the message.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: crate::redact::strip_url_userinfo(&message.into()),
            data: serde_json::Map::new(),
        }
    }

    /// Attach the machine-readable detail of this error.
    #[must_use]
    pub fn with_data(mut self, data: serde_json::Map<String, serde_json::Value>) -> Self {
        self.data = data;
        self
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_code_in_brackets() {
        assert_eq!(
            CliError::new("no-input", "pass paths").to_string(),
            "error[no-input]: pass paths"
        );
    }

    #[test]
    fn message_credentials_are_stripped() {
        let e = CliError::new("x", "GET https://u:p@host/a failed");
        assert_eq!(e.message, "GET https://host/a failed");
    }
}
