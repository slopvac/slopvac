//! The fail-closed gate: after the scrubber has done its best, refuse any text that still looks
//! like a credential or private data.
//!
//! The scrubber is a convenience layer made of heuristics, and heuristics have gaps. This gate is
//! the guarantee: it runs over the final text of every channel (title, each field, the document
//! sample, the body) and answers with a [`Kind`] and nothing else. The caller names the field and
//! the kind, never the matched text, and prints and sends nothing.
//!
//! What counts as sensitive:
//!
//! - **credential**: a GitHub token prefix (even when whitespace splits the token); a 20-character
//!   run of base64-ish characters that mixes letters and digits and is not a plain slug; the value
//!   of any `Authorization` header, including folded continuation lines; `Basic`, `Bearer`,
//!   `Token` or `Digest` followed by a credential-shaped word; any environment value whose name
//!   holds `TOKEN`, `SECRET`, `KEY` or `PASSWORD`.
//! - **repository**: a URL, a leading host or an scp address on a forge host that is not exactly
//!   `slopvac/slopvac`, after percent-decoding, dropping a trailing dot from the host and
//!   resolving `.` and `..` in the path. Protocol-relative `//host/...` counts.
//! - **path**: an absolute Unix path, `~/`, a drive path or a UNC path.
//! - **local name**: the working directory, home directory, user name or host name.
//!
//! The placeholders the scrubber writes (`<token>`, `<path>`, `<org>/<repo>`, `<redacted>`) never
//! trip it. Ordinary prose does not either: `Basic authentication`, `and/or`, a slug such as
//! `claude-3-5-sonnet-20241022`. The cost of a false alarm is a refused run and an edit.

use super::scrub::{has_drive_prefix, percent_decode};

/// Prefixes of GitHub tokens.
const TOKEN_PREFIXES: [&str; 6] = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];
/// Shortest run that can be a secret by shape.
const MIN_RUN: usize = 20;
/// Domains of forges; any host equal to one or under one counts.
const FORGE_DOMAINS: [&str; 4] = [
    "github.com",
    "githubusercontent.com",
    "gitlab.com",
    "bitbucket.org",
];
const OWN_OWNER: &str = "slopvac";
const OWN_REPO: &str = "slopvac";

/// What the gate found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A token, header value or other credential.
    Credential,
    /// A repository or organization that is not slopvac.
    Repository,
    /// An absolute path.
    Path,
    /// A name taken from this machine.
    LocalName,
}

impl Kind {
    /// The words used in the refusal message.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Credential => "credential",
            Self::Repository => "private repository",
            Self::Path => "private path",
            Self::LocalName => "local name",
        }
    }
}

/// Machine-specific strings the gate looks for.
#[derive(Debug, Clone, Default)]
pub struct Gate {
    locals: Vec<String>,
}

impl Gate {
    /// A gate for these local names. It reads no environment: a token is never collected.
    #[must_use]
    pub fn new(locals: Vec<String>) -> Self {
        Self { locals }
    }

    /// The first kind of sensitive content in `text`, or `None`.
    #[must_use]
    pub fn check(&self, text: &str) -> Option<Kind> {
        if Self::credential(text) {
            return Some(Kind::Credential);
        }
        let mut found = None;
        for raw in text.split(is_word_delimiter) {
            let word = raw.trim_matches(['.', ':', '!', '?']);
            if word.is_empty() {
                continue;
            }
            let decoded = percent_decode(word);
            // `\` counts as `/` in a special-scheme URL, so `a\..\b` resolves like `a/../b`.
            let slashed = decoded.replace('\\', "/");
            let candidates = [word, decoded.as_str(), slashed.as_str()];
            for w in candidates {
                match forge_check(w) {
                    Some(true) => continue,
                    Some(false) => return Some(Kind::Repository),
                    None => {}
                }
                if is_absolute_path(w) {
                    found = found.or(Some(Kind::Path));
                }
            }
        }
        if found.is_some() {
            return found;
        }
        self.locals
            .iter()
            .any(|l| text.contains(l.as_str()))
            .then_some(Kind::LocalName)
    }

    /// The check for text as the user typed it, before any scrubbing: only credential shapes. A
    /// partly scrubbed token can leave a fragment, so a raw credential is refused instead.
    #[must_use]
    pub fn check_raw(&self, text: &str) -> Option<Kind> {
        Self::credential(text).then_some(Kind::Credential)
    }

    fn credential(text: &str) -> bool {
        let stripped: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if TOKEN_PREFIXES.iter().any(|p| stripped.contains(p)) {
            return true;
        }
        authorization_value(text)
            || scheme_value(text)
            || text.split(is_word_delimiter).any(|w| {
                // An address on the own repository is not a secret by shape.
                forge_check(w.trim_matches(['.', ':', '!', '?'])) != Some(true) && secret_run(w)
            })
    }
}

fn is_word_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | '<' | '>' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
}

fn is_run_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')
}

/// Whether a `-`/`_` separated run reads as a slug (`claude-3-5-sonnet-20241022`,
/// `x86_64-unknown-linux-gnu`) and not as a secret.
fn slug_like(run: &str) -> bool {
    if run.contains(['+', '/', '=']) {
        return false;
    }
    run.split(['-', '_']).all(|seg| {
        seg.is_empty()
            || seg.bytes().all(|b| b.is_ascii_digit())
            || is_plain_word(seg)
            || (seg.len() <= 8
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
    })
}

/// Letters only, in one case or as one capitalised word.
pub(crate) fn is_plain_word(s: &str) -> bool {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let (first, rest) = s.split_at(1);
    s.bytes().all(|b| b.is_ascii_lowercase())
        || s.bytes().all(|b| b.is_ascii_uppercase())
        || (first.bytes().all(|b| b.is_ascii_uppercase())
            && rest.bytes().all(|b| b.is_ascii_lowercase()))
}

/// Whether a word holds a long run that mixes letters and digits and is not a slug.
///
/// A run without `+` or `=` is judged one `/` segment at a time, so a path such as
/// `/work/project-20241022-report` is a path and not a secret. Base64 with `+` or `=` is judged
/// whole.
fn secret_run(word: &str) -> bool {
    word.split(|c| !is_run_char(c)).any(|run| {
        let pieces: Vec<&str> = if run.contains(['+', '=']) {
            vec![run]
        } else {
            run.split('/').collect()
        };
        pieces.into_iter().any(|piece| {
            piece.len() >= MIN_RUN
                && piece.bytes().any(|b| b.is_ascii_alphabetic())
                && piece.bytes().any(|b| b.is_ascii_digit())
                && !slug_like(piece)
        })
    })
}

fn is_placeholder(value: &str) -> bool {
    value.is_empty() || (value.starts_with('<') && value.ends_with('>'))
}

/// Unfold header continuation lines: a line that starts with space or tab continues the one
/// before it.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        match lines.last_mut() {
            Some(last) if line.starts_with([' ', '\t']) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    lines
}

/// Whether any `Authorization:` line carries a value other than a placeholder.
fn authorization_value(text: &str) -> bool {
    const WORD: &str = "authorization";
    for line in logical_lines(text) {
        let lower = line.to_ascii_lowercase();
        let mut from = 0;
        while let Some(found) = lower[from..].find(WORD) {
            let start = from + found;
            let after = start + WORD.len();
            from = after;
            let boundary = line[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            let rest = line[after..].trim_start_matches(['"', '\'', ' ', '\t']);
            if let (true, Some(value)) = (boundary, rest.strip_prefix(':')) {
                let value = value.trim().trim_matches(['"', '\'', ',', '}']);
                if !is_placeholder(value) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a word reads as ordinary prose: a short word, or a lowercase (or capitalised) word of
/// letters only that has the vowel share and consonant runs of English. A random lowercase
/// credential such as `qwrtpsdfghjkl` fails both tests.
fn prose_word(word: &str) -> bool {
    if word.len() <= 3 {
        return true;
    }
    if !is_plain_word(word) || word.bytes().all(|b| b.is_ascii_uppercase()) {
        return false;
    }
    let lower = word.to_ascii_lowercase();
    let vowels = lower.bytes().filter(|b| b"aeiouy".contains(b)).count();
    let mut run = 0;
    let mut longest = 0;
    for b in lower.bytes() {
        if b"aeiouy".contains(&b) {
            run = 0;
        } else {
            run += 1;
            longest = longest.max(run);
        }
    }
    vowels * 10 >= lower.len() * 3 && longest <= 4
}

/// Whether the word after `Basic`, `Bearer`, `Token` or `Digest` may be a credential. Trailing
/// sentence punctuation is dropped first, so `Basic authentication.` is prose.
fn credential_word(word: &str, min_len: usize) -> bool {
    let word = word.trim_matches(['"', '\'', ',', '.', ';', ':', '!', '?', ')', '(']);
    if word.len() < min_len || word.starts_with('<') {
        return false;
    }
    !prose_word(word)
}

/// Whether a scheme word is followed by a credential, across spaces, tabs and line breaks.
fn scheme_value(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.windows(2).any(|pair| {
        let scheme = pair[0]
            .trim_matches(['"', '\'', ',', ':', '(', ')'])
            .to_ascii_lowercase();
        match scheme.as_str() {
            "bearer" => credential_word(pair[1], 4),
            "basic" | "token" | "digest" => credential_word(pair[1], 6),
            _ => false,
        }
    })
}

fn is_forge_host(host: &str) -> bool {
    FORGE_DOMAINS
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|p| p.ends_with('.')))
}

/// Resolve `.` and `..` and drop empty segments.
fn resolve_segments(path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other.to_owned()),
        }
    }
    out
}

/// For a word that is an address on a forge host: whether it names only the slopvac repository
/// (`Some(true)`) or something else (`Some(false)`). `None` for any other word.
fn forge_check(word: &str) -> Option<bool> {
    let (rest, scp) = if let Some(i) = word.find("://") {
        (&word[i + 3..], false)
    } else if let Some(r) = word.strip_prefix("//") {
        (r, false)
    } else if let Some(at) = word.find('@').filter(|at| !word[..*at].contains('/')) {
        (&word[at + 1..], true)
    } else {
        (word, false)
    };
    let end = rest
        .find(|c| matches!(c, '/' | '?' | '#') || (scp && c == ':'))
        .unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port
        .split(':')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !is_forge_host(&host) {
        return None;
    }
    let path = rest[end..].trim_start_matches(['/', ':']);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let segments = resolve_segments(path);
    let own = |owner: &str, repo: Option<&str>| {
        owner.eq_ignore_ascii_case(OWN_OWNER)
            && repo.is_none_or(|r| r.trim_end_matches(".git").eq_ignore_ascii_case(OWN_REPO))
    };
    Some(match segments.as_slice() {
        [] => true,
        [owner] => owner == "<org>" || own(owner, None),
        [owner, repo, ..] => (owner == "<org>" && repo == "<repo>") || own(owner, Some(repo)),
    })
}

fn is_absolute_path(word: &str) -> bool {
    if word.contains("://") {
        return false;
    }
    word.starts_with("~/")
        || word.starts_with("\\\\")
        || has_drive_prefix(word)
        || word
            .strip_prefix('/')
            .is_some_and(|rest| !rest.starts_with('/') && rest.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> Gate {
        Gate::new(vec!["/work/marker-dir".into(), "marker-user".into()])
    }

    #[test]
    fn credential_shaped_text_is_refused() {
        let g = gate();
        for text in [
            // The reviewers' bypasses.
            "ghp_\nABCDEFGH IJKLMNOPQRSTUVWXYZ0123456789",
            "gh p_ABC",
            "ghp_ab cd ef gh",
            "github_pat_11AAAAAAA0\nabcdefghij",
            "Authorization:\n Basic dXNlcjpwYXNz",
            "authorization: Negotiate YIIabc",
            "Proxy-Authorization: x",
            "{\"Authorization\": \"Bearer abc.def\"}",
            "Basic abCDefGHijKLmnOP",
            "sent Basic dXNlcjpwYXNz now",
            "Token abc123SECRETvalue",
            "Bearer\n  abcd1234",
            "Digest 0123456789abcdef",
            "abc123SECRETvalueXYZ789longer",
            "dGhpcyBpcyBhIHNlY3JldCB0b2tlbg==",
            "abc12345+/abc12345+/abc12345",
            "basic abcdefghijklmnop",
            "bearer\nqwrtpsdfghjklzxcv",
            "Basic\nzxcvbnmqwrtp",
            "ghp_ABCDEFGH ijklmn OPQRST uvwxyz 0123 456789",
        ] {
            assert_eq!(g.check(text), Some(Kind::Credential), "{text:?}");
        }
    }

    #[test]
    fn forge_addresses_other_than_slopvac_are_refused() {
        let g = gate();
        for text in [
            "https://github.com/private/repo",
            "//github.com/private/repo",
            "github.com./private/repo",
            "https://GITHUB.com./private/repo/issues",
            "https://github.com/%70rivate/repo",
            "https://github.com/priv%61te%2Frepo",
            "https://github.com/slopvac/slopvac/../../private/repo",
            "https://github.com/slopvac/other",
            "https://github.com/acme",
            "git@github.com:acme/secret.git",
            "ssh://git@gitlab.com/acme/x",
            "https://gist.github.com/acme/abc",
            "https://raw.githubusercontent.com/acme/x/main/f",
            "https://api.github.com/repos/acme/x",
            "https://bitbucket.org/acme/x",
            "https://github.com/slopvac/slopvac\\..\\..\\private\\repo",
            "https://github.com/srobroek\\slopvac/../../private/repo",
        ] {
            assert_eq!(g.check(text), Some(Kind::Repository), "{text:?}");
        }
    }

    #[test]
    fn absolute_paths_and_local_names_are_refused() {
        let g = gate();
        for (text, kind) in [
            ("/etc/hosts", Kind::Path),
            ("see ~/notes/todo", Kind::Path),
            (r"C:\Users\bob\x", Kind::Path),
            ("D:/work/x", Kind::Path),
            (r"\\fileserver\share", Kind::Path),
            ("in /work/marker-dir here", Kind::Path),
            ("user marker-user here", Kind::LocalName),
        ] {
            assert_eq!(g.check(text), Some(kind), "{text:?}");
        }
    }

    #[test]
    fn placeholders_prose_and_the_own_repository_pass() {
        let g = gate();
        for text in [
            "<token>",
            "Authorization: <redacted>",
            "Bearer <token>",
            "Basic <token> and Token <token>",
            "ran in <path> as <user> on <host>",
            "https://github.com/<org>/<repo>",
            "https://github.com/<org>",
            "git@github.com:<org>/<repo>",
            "https://github.com/slopvac/slopvac",
            "https://github.com/slopvac/slopvac/issues/12",
            "https://github.com/slopvac/slopvac.git",
            "git@github.com:slopvac/slopvac.git",
            "https://www.github.com/slopvac/slopvac/pull/9",
            "https://github.com./slopvac/slopvac/./issues",
            "https://github.com/slopvac/slopvac/../slopvac/issues",
            "basic internationalization support",
            "Basic authentication works",
            "Basic Internationalization",
            "basic internationalization support",
            "Basic authentication.",
            "Basic authentication, then Token expiry!",
            "(Bearer tokens are short-lived)",
            "Digest: a summary of changes.",
            "Token count is 5",
            "the token budget and bearer of news",
            "Digest of changes",
            "read/write, input/output, and/or on/off, TCP/IP",
            "the authorization process is slow",
            "authorization is required",
            "claude-3-5-sonnet-20241022",
            "x86_64-unknown-linux-gnu aarch64-apple-darwin",
            "gpt-4o-2024-08-06",
            "https://example.com/github.com/guide",
            "https://docs.example.com/gitlab.com/acme/x",
            "visit github.com for more",
            "https://github.com.evil.example/acme/x",
            "version 3.0.0 on macOS 15.1",
            "a slash command /model works",
            "see the \"short\" tokens: abc, a1b2",
            "abcdefghijklmnopqrstuvwxyz and 0123456789012345678901",
        ] {
            assert_eq!(g.check(text), None, "{text:?}");
        }
    }
}
