//! The `report` command's error codes, as a message catalog.
//!
//! Every failure of `slopvac report` exits 2 with one of these codes. The parameters hold fixed
//! phrases and form field ids. They never hold what the user typed or the contents of a file.

use slopvac_messages::Catalog;

/// Why `slopvac report` stopped.
#[derive(Debug, Catalog)]
pub enum ReportError {
    /// A required form field is missing.
    #[code("report-incomplete")]
    #[message(
        "required fields are missing: {fields}. Pass each as --field ID=VALUE (tick a box with ID=true)."
    )]
    Incomplete {
        /// The ids of the missing fields.
        fields: String,
    },
    /// The `--body-file` path cannot be used or written.
    #[code("report-body-file")]
    #[message("--body-file: {reason}")]
    BodyFile {
        /// A fixed phrase that names the problem.
        reason: String,
    },
    /// The GitHub CLI (`gh`) was not found.
    #[code("report-gh-missing")]
    #[message(
        "the GitHub CLI `gh` was not found on PATH. Nothing was created. Install it from https://cli.github.com/, or run the printed command yourself."
    )]
    GhMissing,
    /// The GitHub CLI is not logged in to github.com.
    #[code("report-gh-not-logged-in")]
    #[message("`gh auth status` failed; run `gh auth login`. Nothing was created.")]
    GhNotLoggedIn,
    /// `gh issue create` failed or ran longer than 60 seconds.
    #[code("report-gh-failed")]
    #[message("`gh issue create` failed: {reason}")]
    GhFailed {
        /// The exit status, or that the command timed out.
        reason: String,
    },
    /// A title, field or document sample holds a control character, or an argument is malformed.
    #[code("report-invalid")]
    #[message("invalid report input: {detail}")]
    Invalid {
        /// A fixed phrase that names the problem and the field id, never the value.
        detail: String,
    },
    /// `--gh --yes` needs the network and the run is offline.
    #[code("offline-network-needed")]
    #[message("offline: report --gh --yes needs the network")]
    OfflineNetworkNeeded,
    /// The `--include-text` file cannot be read.
    #[code("report-sample-unreadable")]
    #[message("--include-text: {reason}")]
    SampleUnreadable {
        /// A fixed phrase that names the problem, never the file contents.
        reason: String,
    },
}

impl From<ReportError> for crate::errors::CliError {
    /// The code and the text both come from the catalog entry.
    fn from(error: ReportError) -> Self {
        Self::new(error.code(), slopvac_messages::render(&error))
    }
}
