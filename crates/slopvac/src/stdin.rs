//! `lint --stdin`: option checks, language choice and the UTF-8 gate
//! (docs/contract/policies.md, 1.7; compat rows `behavior:stdin-*`).

use std::io::Read;

use clap::ArgMatches;

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
const REJECTED: [(&str, &str); 4] = [
    ("diff-base", ""),
    ("diff-working-tree", ""),
    ("fix", " because stdin has no file to write back to"),
    ("open", ""),
];

/// What `--stdin` resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdinPlan {
    /// `code-comments` or `prose`.
    pub mode: &'static str,
    /// The parser name.
    pub language: String,
}

fn usage(message: impl Into<String>) -> CliError {
    CliError::new("usage", message)
}

/// Validate the options given with `--stdin`.
///
/// # Errors
/// A `usage` error for targets, file-only options, or a language that does
/// not fit the mode.
pub fn plan(m: &ArgMatches) -> Result<StdinPlan, CliError> {
    if m.get_many::<std::path::PathBuf>("targets").is_some() {
        return Err(usage("PATH must be empty when --stdin is set."));
    }
    for (name, reason) in REJECTED {
        let given = m.try_get_one::<bool>(name).ok().flatten().copied() == Some(true)
            || m.try_get_one::<String>(name).ok().flatten().is_some();
        if given {
            return Err(usage(format!(
                "--{name} is not supported with --stdin{reason}."
            )));
        }
    }
    let code_comments = m.get_one::<String>("mode").map(String::as_str) == Some("code-comments");
    let language = m.get_one::<String>("language");
    if code_comments && language.is_none() {
        return Err(usage(
            "--language is required with --stdin --mode code-comments.",
        ));
    }
    let language = language.map_or("markdown", String::as_str);
    let (mode, valid): (&'static str, &[&str]) = if code_comments {
        ("code-comments", &COMMENT_LANGUAGES)
    } else {
        ("prose", &PROSE_LANGUAGES)
    };
    if !valid.contains(&language) {
        return Err(usage(format!(
            "--language {language:?} is not valid for mode {mode}. Valid values: {}.",
            valid.join(", ")
        )));
    }
    Ok(StdinPlan {
        mode,
        language: language.to_owned(),
    })
}

/// Read the whole document and require UTF-8.
///
/// # Errors
/// `input-unreadable` when the read fails, `input-not-utf8` with the offset
/// of the first bad byte otherwise.
pub fn read(source: &mut dyn Read) -> Result<Vec<u8>, CliError> {
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).map_err(|e| {
        CliError::new(
            "input-unreadable",
            format!("{STDIN_NAME}: cannot read: {e}"),
        )
    })?;
    if let Err(err) = std::str::from_utf8(&buf) {
        return Err(CliError::new(
            "input-not-utf8",
            format!("{STDIN_NAME}: not valid UTF-8 (byte {})", err.valid_up_to()),
        ));
    }
    Ok(buf)
}
