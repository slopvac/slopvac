//! The text of `slopvac init`: the lines it prints and the messages of its refusals.

use slopvac_messages::Catalog;

/// What `slopvac init` prints and why it stops.
#[derive(Debug, Catalog)]
pub(super) enum InitText {
    /// The config file was written.
    #[code("cli-init-wrote")]
    #[text("wrote {path}")]
    Wrote {
        /// The path as shown to the user.
        #[display_path]
        path: String,
    },
    /// A config file was already there and `--force` was not set.
    #[code("cli-init-kept")]
    #[text("{path} exists; keeping it.")]
    Kept {
        /// The path as shown to the user.
        #[display_path]
        path: String,
    },
    /// A house style detected in the project.
    #[code("cli-init-house-style")]
    #[text("house style: {description}")]
    HouseStyle {
        /// The detector's own description of the style.
        description: String,
    },
    /// The harnesses init sets up.
    #[code("cli-init-harnesses")]
    #[text("harnesses: {names}")]
    Harnesses {
        /// The harness names, comma separated.
        names: String,
    },
    /// The last line of a successful run.
    #[code("cli-init-next")]
    #[text("next: slopvac prime")]
    Next,
    /// `--path` names a directory.
    #[code("cli-init-directory-path")]
    #[text("--path {path} is a directory.")]
    DirectoryPath {
        /// The path as shown to the user.
        #[display_path]
        path: String,
    },
    /// `--skip-agents` was given with an option that sets up agents.
    #[code("cli-init-skip-agents-conflict")]
    #[text("--skip-agents writes no steering; drop --harness and --no-skills.")]
    SkipAgentsConflict,
    /// The config file could not be written.
    #[code("cli-init-write-failed")]
    #[text("{path}: cannot write: {kind}")]
    WriteFailed {
        /// The path as shown to the user.
        #[display_path]
        path: String,
        /// The kind of the I/O error.
        kind: String,
    },
    /// A line could not be written to the output stream.
    #[code("cli-init-output-failed")]
    #[text("cannot write output: {kind}")]
    OutputFailed {
        /// The kind of the I/O error.
        kind: String,
    },
}
