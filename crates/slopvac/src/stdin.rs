//! `lint --stdin`: option checks, language choice and the UTF-8 gate
//! (docs/contract/policies.md, 1.7; compat rows `behavior:stdin-*`).

use std::io::Read;

use clap::ArgMatches;

use crate::cli_messages::IoMessage;
use crate::errors::CliError;
use crate::redact::STDIN_NAME;

/// Languages `--language` accepts in prose mode.
pub const PROSE_LANGUAGES: [&str; 5] = ["html", "markdown", "mdx", "rst", "text"];

/// Languages `--language` accepts in `code-comments` mode.
pub const COMMENT_LANGUAGES: [&str; 30] = [
    "c",
    "clojure",
    "cpp",
    "csharp",
    "css",
    "dart",
    "elixir",
    "fsharp",
    "go",
    "haskell",
    "java",
    "javascript",
    "julia",
    "kotlin",
    "less",
    "lua",
    "perl",
    "php",
    "powershell",
    "proto",
    "python",
    "r",
    "ruby",
    "rust",
    "scala",
    "scss",
    "shell",
    "swift",
    "toml",
    "typescript",
];

/// Options that make no sense without a file on disk.
const REJECTED: [&str; 4] = ["diff-base", "diff-working-tree", "fix", "open"];

/// What `--stdin` resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdinPlan {
    /// `code-comments` or `prose`.
    pub mode: &'static str,
    /// The parser name.
    pub language: String,
}

/// Validate the options given with `--stdin`.
///
/// # Errors
/// A `usage` error for targets, file-only options, or a language that does
/// not fit the mode.
pub fn plan(m: &ArgMatches) -> Result<StdinPlan, CliError> {
    if m.get_many::<std::path::PathBuf>("targets").is_some() {
        return Err(IoMessage::usage(&IoMessage::StdinPathGiven));
    }
    for name in REJECTED {
        let given = m.try_get_one::<bool>(name).ok().flatten().copied() == Some(true)
            || m.try_get_one::<String>(name).ok().flatten().is_some();
        if given {
            let sentence = if name == "fix" {
                IoMessage::StdinFixUnsupported
            } else {
                IoMessage::StdinOptionUnsupported {
                    option: name.to_owned(),
                }
            };
            return Err(IoMessage::usage(&sentence));
        }
    }
    let code_comments = m.get_one::<String>("mode").map(String::as_str) == Some("code-comments");
    let language = m.get_one::<String>("language");
    if code_comments && language.is_none() {
        return Err(IoMessage::usage(&IoMessage::StdinLanguageRequired));
    }
    let language = language.map_or("markdown", String::as_str);
    let (mode, valid): (&'static str, &[&str]) = if code_comments {
        ("code-comments", &COMMENT_LANGUAGES)
    } else {
        ("prose", &PROSE_LANGUAGES)
    };
    if !valid.contains(&language) {
        return Err(IoMessage::usage(&IoMessage::StdinLanguageInvalid {
            language: format!("{language:?}"),
            mode: mode.to_owned(),
            valid: valid.join(", "),
        }));
    }
    Ok(StdinPlan {
        mode,
        language: language.to_owned(),
    })
}

/// Read the whole document under the document size cap and require UTF-8.
///
/// # Errors
/// `document-too-large` when the input is over the cap (nothing is truncated),
/// `input-unreadable` when the read fails, `input-not-utf8` with the offset
/// of the first bad byte otherwise.
pub fn read(source: &mut dyn Read) -> Result<Vec<u8>, CliError> {
    let buf =
        slopvac_fs::read_capped(source, slopvac_fs::MAX_DOCUMENT_BYTES).map_err(
            |err| match err {
                slopvac_fs::Error::CapExceeded { .. } => IoMessage::stdin_too_large(STDIN_NAME),
                _ => IoMessage::input_unreadable(&IoMessage::StdinNotReadable {
                    shown: STDIN_NAME.to_owned(),
                }),
            },
        )?;
    if let Err(err) = std::str::from_utf8(&buf) {
        return Err(IoMessage::input_not_utf8(&IoMessage::StdinNotUtf8 {
            shown: STDIN_NAME.to_owned(),
            offset: err.valid_up_to(),
        }));
    }
    Ok(buf)
}
