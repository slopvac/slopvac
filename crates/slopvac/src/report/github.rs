//! The prefilled issue-form URL and the browser.

use std::fmt::Write as _;
use std::process::{Command, Stdio};

use super::issue::Report;
use super::{Category, Kind, REPOSITORY};

/// Longest prefilled URL. GitHub answers 414 above its limit; this stays well under it.
pub const MAX_URL_BYTES: usize = 7000;

/// Opens a URL in a browser.
pub trait Opener {
    /// Start the browser. Does not wait for it.
    fn open(&self, url: &str) -> std::io::Result<()>;
}

/// `open` on macOS, `xdg-open` elsewhere. Started without a shell, with the URL as one argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, url: &str) -> std::io::Result<()> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
    }
}

/// A prefilled issue-form URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormUrl {
    /// The URL.
    pub url: String,
    /// Field ids the URL could not hold.
    pub omitted: Vec<&'static str>,
}

/// Percent-encode everything outside the RFC 3986 unreserved set.
#[must_use]
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The issue-form URL with the title and answers filled in by field id.
///
/// It carries no `labels`: GitHub answers 404 when the reader may not set labels, and the form
/// template already sets them.
///
/// Checkbox fields stay empty: a tick is the reporter's decision. Fields are added in form order;
/// one that would push the URL past [`MAX_URL_BYTES`] is left out and named in `omitted`.
#[must_use]
pub fn prefilled_url(category: &Category, report: &Report) -> FormUrl {
    let mut url = format!(
        "https://github.com/{REPOSITORY}/issues/new?template={}&title={}",
        encode(category.form_file),
        encode(&report.title)
    );
    let mut omitted = Vec::new();
    for field in &category.fields {
        if field.kind == Kind::Checkboxes {
            continue;
        }
        let Some(values) = report.answers.get(field.id) else {
            continue;
        };
        let piece = format!("&{}={}", field.id, encode(&values.join(", ")));
        if url.len() + piece.len() > MAX_URL_BYTES {
            omitted.push(field.id);
        } else {
            url.push_str(&piece);
        }
    }
    FormUrl { url, omitted }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_keeps_only_unreserved_bytes() {
        assert_eq!(encode("a b&c=d/é~-_."), "a%20b%26c%3Dd%2F%C3%A9~-_.");
    }
}
