//! The messages of the CLI's input and output layer: the general error codes and the text of
//! `--stdin` and `--out`.
//!
//! [`IoMessage`] holds the codes that many commands share, each with the detail of its site, which
//! never holds document text or an absolute path outside the project root. It also holds the
//! sentences that fill those details. The binary registers it in [`crate::messages::ALL`].

use slopvac_messages::{Catalog, Entry};

use crate::errors::CliError;

/// The general error codes of the CLI, each of which exits 2, and the sentences behind their
/// details.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum IoMessage {
    /// An argument value or a combination of options that slopvac refuses after parsing.
    #[code("usage")]
    #[message("{detail}")]
    Usage {
        /// What is wrong and how to fix it.
        detail: String,
    },
    /// A 2.x option that 3.0 removed.
    #[expect(dead_code, reason = "registered here; `init` and `run` adopt it in their own changes")]
    #[code("removed-option")]
    #[message("{detail}")]
    RemovedOption {
        /// The pointer to what replaces the option.
        detail: String,
    },
    /// The output cannot be rendered in the requested format.
    #[code("render-failed")]
    #[message("{detail}")]
    RenderFailed {
        /// Why the renderer refused.
        detail: String,
    },
    /// The output cannot be written.
    #[code("output-failed")]
    #[message("{detail}")]
    OutputFailed {
        /// What could not be written.
        detail: String,
    },
    /// An input cannot be read.
    #[code("input-unreadable")]
    #[message("{detail}")]
    InputUnreadable {
        /// Which input, as shown to the user.
        detail: String,
    },
    /// An input is not valid UTF-8.
    #[code("input-not-utf8")]
    #[message("{detail}")]
    InputNotUtf8 {
        /// Which input and where the first bad byte is.
        detail: String,
    },

    /// `--stdin` was given with paths.
    #[code("cli-stdin-path-given")]
    #[text("PATH must be empty when --stdin is set.")]
    StdinPathGiven,
    /// `--stdin` was given with an option that needs a file on disk.
    #[code("cli-stdin-option-unsupported")]
    #[text("--{option} is not supported with --stdin.")]
    StdinOptionUnsupported {
        /// The option name, one of a fixed list.
        option: String,
    },
    /// `--stdin` was given with `--fix`.
    #[code("cli-stdin-fix-unsupported")]
    #[text("--fix is not supported with --stdin because stdin has no file to write back to.")]
    StdinFixUnsupported,
    /// `--stdin --mode code-comments` has no `--language`.
    #[code("cli-stdin-language-required")]
    #[text("--language is required with --stdin --mode code-comments.")]
    StdinLanguageRequired,
    /// `--language` does not fit the mode.
    #[code("cli-stdin-language-invalid")]
    #[text("--language {language} is not valid for mode {mode}. Valid values: {valid}.")]
    StdinLanguageInvalid {
        /// The value as the user wrote it, debug-quoted.
        language: String,
        /// The mode, one of two fixed names.
        mode: String,
        /// The accepted languages, comma separated.
        valid: String,
    },
    /// Standard input is over the document size cap.
    #[code("cli-stdin-document-too-large")]
    #[text("{shown}: larger than the 64 MiB document limit")]
    StdinDocumentTooLarge {
        /// The name of the input.
        shown: String,
    },
    /// Reading standard input failed.
    #[code("cli-stdin-not-readable")]
    #[text("{shown}: cannot read")]
    StdinNotReadable {
        /// The name of the input.
        shown: String,
    },
    /// Standard input is not UTF-8.
    #[code("cli-stdin-not-utf8")]
    #[text("{shown}: not valid UTF-8 (byte {offset})")]
    StdinNotUtf8 {
        /// The name of the input.
        shown: String,
        /// The offset of the first bad byte.
        #[count]
        offset: usize,
    },
    /// `--open` was combined with a format other than HTML.
    #[code("cli-open-needs-html")]
    #[text("--open shows an HTML report; drop --format or use --format html.")]
    OpenNeedsHtml,
    /// Writing the report failed.
    #[code("cli-report-write-failed")]
    #[text("cannot write the report: {kind}")]
    ReportWriteFailed {
        /// The kind of the I/O error, in the system's words.
        kind: String,
    },
    /// `--open` did nothing because CI is on.
    #[code("cli-open-skipped-ci")]
    #[text("not opening the report: CI is set.")]
    OpenSkippedCi,
    /// The opener failed. The report is already written.
    #[code("cli-open-failed")]
    #[text("warning: could not open the report: {detail}")]
    OpenFailed {
        /// The opener's own failure text.
        detail: String,
    },
    /// No opener is known for this platform.
    #[code("cli-open-no-opener")]
    #[text("no opener is known for this platform")]
    OpenNoOpener,
    /// The opener program ran and failed.
    #[code("cli-open-exited")]
    #[text("{program} exited with {status}")]
    OpenExited {
        /// The opener program, `open` or `xdg-open`.
        program: String,
        /// The exit status, in the system's words.
        status: String,
    },
}

impl IoMessage {
    /// The rendered sentence, for the `detail` of an error.
    pub(crate) fn text(&self) -> String {
        slopvac_messages::render(self)
    }

    /// An error with this entry's code and its text.
    fn into_error(self) -> CliError {
        CliError::new(self.code(), slopvac_messages::render(&self))
    }

    /// A `usage` error whose detail is `sentence`.
    pub(crate) fn usage(sentence: &Self) -> CliError {
        Self::Usage {
            detail: sentence.text(),
        }
        .into_error()
    }

    /// An `output-failed` error whose detail is `sentence`.
    pub(crate) fn output_failed(sentence: &Self) -> CliError {
        Self::OutputFailed {
            detail: sentence.text(),
        }
        .into_error()
    }

    /// A `render-failed` error whose detail is `detail`, the renderer's own text.
    pub(crate) fn render_failed(detail: String) -> CliError {
        Self::RenderFailed { detail }.into_error()
    }

    /// An `input-unreadable` error whose detail is `sentence`.
    pub(crate) fn input_unreadable(sentence: &Self) -> CliError {
        Self::InputUnreadable {
            detail: sentence.text(),
        }
        .into_error()
    }

    /// An `input-not-utf8` error whose detail is `sentence`.
    pub(crate) fn input_not_utf8(sentence: &Self) -> CliError {
        Self::InputNotUtf8 {
            detail: sentence.text(),
        }
        .into_error()
    }

    /// The `document-too-large` error of standard input. The code is the one `slopvac-config`
    /// owns.
    pub(crate) fn stdin_too_large(shown: &str) -> CliError {
        CliError::new(
            slopvac_config::Code::DocumentTooLarge.as_str(),
            Self::StdinDocumentTooLarge {
                shown: shown.to_owned(),
            }
            .text(),
        )
    }
}

/// Entries of [`IoMessage`], for the registry.
pub(crate) fn entries() -> &'static [Entry] {
    <IoMessage as Catalog>::entries()
}
