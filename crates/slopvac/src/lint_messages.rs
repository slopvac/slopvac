//! The `slopvac lint` option errors, as a message catalog.

use slopvac_messages::Catalog;

/// Why `slopvac lint` refused its options.
#[derive(Debug, Catalog)]
pub enum LintOptionError {
    /// `--explain-config` was given with a report format other than `text`.
    #[code("explain-config-format-unsupported")]
    #[message("`--explain-config` prints text only; remove `--format {format}`.")]
    ExplainConfigFormat {
        /// The `--format` value from the command line, one of the fixed format names.
        format: String,
    },
    /// `--explain-config` was given with an option it would ignore.
    #[code("explain-config-option-unsupported")]
    #[message("`--explain-config` cannot be used with `{option}`; remove it.")]
    ExplainConfigOption {
        /// The option as written, one of a fixed list of flag names.
        option: String,
    },
    /// `styles` in the project config lists a style that does not ship.
    #[code("unknown-style")]
    #[message("slopvac.toml: unknown style {name}; the styles that ship are: {shipped}.")]
    UnknownStyle {
        /// The listed name, quoted.
        name: String,
        /// The names of the shipped styles, comma separated.
        shipped: String,
    },
}

impl From<LintOptionError> for crate::errors::CliError {
    /// The code and the text both come from the catalog entry.
    fn from(error: LintOptionError) -> Self {
        Self::new(error.code(), slopvac_messages::render(&error))
    }
}

/// What `slopvac lint --fix` says about a file it did not fix.
#[derive(Debug, Catalog)]
pub enum LintFixMessage {
    /// The engine refused to write the file. The run records the refusal's own code.
    #[code("fix-refused")]
    #[text("{shown}: not fixed ({reason})")]
    Refused {
        /// The path as shown in the report.
        shown: String,
        /// The refusal code from the engine, one of a fixed list.
        reason: String,
    },
    /// The fix failed for a reason other than a lint error or a refusal.
    #[code("fix-failed")]
    #[message("{shown}: not fixed: {reason}")]
    Failed {
        /// The path as shown in the report.
        shown: String,
        /// What went wrong, without the path.
        reason: String,
    },
}

/// Why `slopvac lint --fix` could not fix a file, as the `reason` of [`LintFixMessage::Failed`].
#[derive(Debug, Catalog)]
pub enum LintFixReason {
    /// The file is not UTF-8.
    #[code("lint-fix-reason-not-utf8")]
    #[text("not valid UTF-8")]
    NotUtf8,
    /// The check for an access control list could not run.
    #[code("lint-fix-reason-acl-check")]
    #[text("the access control list check failed")]
    AclCheck,
    /// The project root was replaced while the run opened it.
    #[code("lint-fix-reason-root-changed")]
    #[text("the project root was replaced while it was opened")]
    RootChanged,
    /// A read or write failed, with the system's own wording.
    #[code("lint-fix-reason-io")]
    #[text("{detail}")]
    Io {
        /// The system's description of the failure, without the path.
        detail: String,
    },
}
