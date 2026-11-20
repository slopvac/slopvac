//! The `slopvac lint` option errors, as a message catalog.

use slopvac_messages::Catalog;

/// Why `slopvac lint` refused its options.
#[derive(Debug, Catalog)]
pub enum LintOptionError {
    /// `--explain-config` was given with a report format other than `text`.
    #[code("explain-config-format")]
    #[message("`--explain-config` prints text only; remove `--format {format}`.")]
    ExplainConfigFormat {
        /// The `--format` value from the command line, one of the fixed format names.
        format: String,
    },
}

impl From<LintOptionError> for crate::errors::CliError {
    /// The code and the text both come from the catalog entry.
    fn from(error: LintOptionError) -> Self {
        Self::new(error.code(), slopvac_messages::render(&error))
    }
}
