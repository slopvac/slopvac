//! Remove what must not leave the machine from an issue report.
//!
//! `CONTRIBUTING.md` bars paths, file names, user names, host names, repository and organization
//! names and other private data from reports. This module cleans three ways:
//!
//! 1. exact strings the machine knows: the working directory, the home directory, the user name
//!    and the host name;
//! 2. path-shaped words and file names (see [`is_path_like`]);
//! 3. secrets: tokens, `Bearer` credentials and the user-info of URLs.
//!
//! Document text the user chose to include (`--include-text`) gets steps 1 and 3 only, because
//! step 2 would rewrite the sample.

use std::path::Path;

use super::gate::{Gate, Kind};
use crate::env::Env;
use crate::redact::strip_url_userinfo;

/// File name extensions that make a bare word a file name.
const FILE_EXTENSIONS: [&str; 27] = [
    "md", "mdx", "rst", "txt", "html", "htm", "toml", "json", "yaml", "yml", "py", "rs", "js",
    "ts", "tsx", "jsx", "go", "c", "h", "cpp", "java", "sh", "lock", "cfg", "ini", "conf", "log",
];

/// Names that carry no private information and that the report workflow uses.
const ALLOWED_NAMES: [&str; 2] = ["sample.md", "slopvac.toml"];

/// Token prefixes GitHub issues.
const TOKEN_PREFIXES: [&str; 6] = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];

/// Characters that end a word for path detection.
fn is_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '<'
                | '>'
                | ','
                | ';'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | '='
        )
}

fn has_file_extension(word: &str) -> bool {
    let name = word.rsplit('/').next().unwrap_or(word);
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && FILE_EXTENSIONS.contains(&ext))
}

/// Whether a word looks like `C:\dir` or `C:/dir`.
pub(crate) fn has_drive_prefix(word: &str) -> bool {
    let b = word.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Whether a word is a path or a file name.
///
/// URLs are not paths. A word that starts with `/` needs a second `/`, so a slash command such as
/// `/model` stays. `~/`, `./` and `../` words, drive paths (`C:\x`) and UNC paths (`\\host\x`)
/// are paths. A relative path with two or more separators is a path; with one separator it is a
/// path when a side carries a `.`, `_`, `-` or digit (`my-repo/src`, `v1/api`), which leaves
/// `and/or` and `yes/no` alone. Any other word with a known file extension is a file name,
/// except the names in `ALLOWED_NAMES`.
#[must_use]
pub fn is_path_like(word: &str) -> bool {
    if word.contains("://") || ALLOWED_NAMES.contains(&word) {
        return false;
    }
    if word.starts_with("\\\\") || has_drive_prefix(word) {
        return true;
    }
    // A name with a backslash between two parts is a path: `acme\notes`. This comes before the
    // prose exception for a single `/`, because `\` is not used in prose.
    if word
        .char_indices()
        .any(|(i, c)| c == '\\' && i > 0 && i + 1 < word.len())
    {
        return true;
    }
    // Compare with `/` as the only separator from here on.
    let word = &word.replace('\\', "/");
    let word = word.as_str();
    if word.starts_with("~/") || word.starts_with("./") || word.starts_with("../") {
        return true;
    }
    if let Some(rest) = word.strip_prefix('/') {
        return rest.contains('/') || has_file_extension(word);
    }
    let separators = word.matches('/').count();
    if separators >= 2 {
        return true;
    }
    if separators == 1 {
        let marked = word
            .chars()
            .any(|c| matches!(c, '.' | '_' | '-') || c.is_ascii_digit());
        // A slash at either end (`input/`, `/x`) is not a path between two names.
        let both_sides = !word.starts_with('/') && !word.ends_with('/');
        return both_sides && (marked || has_file_extension(word));
    }
    has_file_extension(word)
}

/// Replace path-like words with `<path>`.
#[must_use]
pub fn scrub_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        // Sentence punctuation after a path is not part of it.
        let trimmed = word.trim_end_matches(['.', ':', '!', '?']);
        let tail = &word[trimmed.len()..];
        if is_path_like(trimmed) {
            out.push_str("<path>");
            out.push_str(tail);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if is_delimiter(c) {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn byte_len(chars: impl Iterator<Item = char>) -> usize {
    chars.map(char::len_utf8).sum()
}

/// How many characters follow each token prefix in a real token.
fn token_suffix_len(prefix: &str) -> usize {
    if prefix == "github_pat_" { 82 } else { 36 }
}

/// Whether a run of token characters, found after whitespace, continues a wrapped token.
///
/// Prose words must not qualify. A random token piece has a digit or both cases, or it ends the
/// token exactly.
fn continues_token(run: &str, have: usize, target: usize) -> bool {
    let digit = run.bytes().any(|b| b.is_ascii_digit());
    let mixed =
        run.bytes().any(|b| b.is_ascii_uppercase()) && run.bytes().any(|b| b.is_ascii_lowercase());
    (run.len() >= 6 && (digit || mixed)) || have + run.len() == target
}

/// Replace `prefix` and the token after it, even when whitespace or a line break splits the token.
fn redact_prefixed(text: &str, prefix: &str) -> String {
    let target = token_suffix_len(prefix);
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        out.push_str(&rest[..at]);
        let after = &rest[at + prefix.len()..];
        let (mut pos, mut count, mut end) = (0, 0, 0);
        loop {
            let run_len = byte_len(after[pos..].chars().take_while(|c| is_token_char(*c)));
            if run_len == 0 {
                break;
            }
            let run = &after[pos..pos + run_len];
            if pos > 0 && !continues_token(run, count, target) {
                break;
            }
            count += run_len;
            pos += run_len;
            end = pos;
            if count >= target {
                break;
            }
            let gap = byte_len(after[pos..].chars().take_while(|c| c.is_whitespace()));
            if gap == 0 {
                break;
            }
            pos += gap;
        }
        if count >= 8 {
            out.push_str("<token>");
            rest = &after[end..];
        } else {
            out.push_str(prefix);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Replace the value of every `Authorization:` header line, whatever the scheme.
fn redact_authorization(text: &str) -> String {
    const WORD: &str = "authorization";
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(WORD) {
        let start = at + found;
        let after_word = start + WORD.len();
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        // Allow a closing quote and spaces before the colon: `"Authorization": "..."`.
        let tail = &text[after_word..];
        let skipped = byte_len(
            tail.chars()
                .take_while(|c| matches!(c, '"' | '\'' | ' ' | '\t')),
        );
        if !boundary || !tail[skipped..].starts_with(':') {
            out.push_str(&text[at..after_word]);
            at = after_word;
            continue;
        }
        let colon_end = after_word + skipped + 1;
        out.push_str(&text[at..colon_end]);
        out.push_str(" <redacted>");
        at = text[colon_end..]
            .find('\n')
            .map_or(text.len(), |i| colon_end + i);
    }
    out.push_str(&text[at..]);
    out
}

/// Whether a credential after `Basic`, `Token` or `Digest` is more than an ordinary word. It
/// agrees with the gate: eight characters or more that are not one plain word.
fn looks_like_credential(c: &str) -> bool {
    c.len() >= 8 && !super::gate::is_plain_word(c)
}

/// Replace `<scheme> <credential>` with `<Scheme> <token>`. `strict` limits the match to
/// credential-shaped words, so prose such as "Token count" stays.
fn redact_scheme(text: &str, word: &str, canonical: &str, strict: bool) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(word) {
        let start = at + found;
        let after_word = start + word.len();
        out.push_str(&text[at..after_word]);
        at = after_word;
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        let tail = &text[after_word..];
        // The separator is a run of spaces or tabs, so `Bearer   abc` counts.
        if !before_ok || !tail.starts_with([' ', '\t']) {
            continue;
        }
        let padding = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        let credential = &tail[padding..];
        let len = byte_len(credential.chars().take_while(|c| !c.is_whitespace()));
        if len == 0 || (strict && !looks_like_credential(&credential[..len])) {
            continue;
        }
        out.truncate(out.len() - word.len());
        out.push_str(canonical);
        out.push_str(" <token>");
        at = after_word + padding + len;
    }
    out.push_str(&text[at..]);
    out
}

/// Replace GitHub tokens, `Authorization` headers and `Bearer`, `Basic`, `Token` and `Digest`
/// credentials, and strip URL user-info.
#[must_use]
pub fn scrub_secrets(text: &str) -> String {
    let mut out = strip_url_userinfo(text);
    for prefix in TOKEN_PREFIXES {
        out = redact_prefixed(&out, prefix);
    }
    out = redact_authorization(&out);
    for (word, canonical, strict) in [
        ("bearer", "Bearer", false),
        ("basic", "Basic", true),
        ("token", "Token", true),
        ("digest", "Digest", true),
    ] {
        out = redact_scheme(&out, word, canonical, strict);
    }
    out
}

/// The only repository a report may name.
const OWN_OWNER: &str = "slopvac";
const OWN_REPO: &str = "slopvac";
/// Hosts whose `owner/repo` paths identify a private repository or organization.
const FORGE_HOSTS: [&str; 4] = [
    "github.com",
    "www.github.com",
    "gitlab.com",
    "bitbucket.org",
];

/// Decode `%XX` escapes. Invalid escapes stay as they are.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'%'));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_repo_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | '<' | '>' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
}

/// Rewrite one word that names a repository on a forge host. `None` leaves the word alone.
///
/// Only a URL authority (`https://github.com/...`, with optional user-info and port), a bare
/// leading host (`github.com/...`) and the scp form (`git@github.com:...`) count. The same host
/// elsewhere in a word, such as `https://example.com/github.com/guide`, does not.
fn forge_word(word: &str) -> Option<String> {
    let (host_start, scp) = if let Some(i) = word.find("://") {
        (i + 3, false)
    } else if let Some(at) = word.find('@') {
        (at + 1, true)
    } else {
        (0, false)
    };
    let rest = &word[host_start..];
    // Drop user-info in a URL authority (`user@host`); the scp form already skipped it.
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host_off = if scp {
        0
    } else {
        rest[..authority_end].rfind('@').map_or(0, |i| i + 1)
    };
    let from_host = &rest[host_off..];
    let host_len = from_host
        .find(['/', ':', '?', '#'])
        .unwrap_or(from_host.len());
    let host = &from_host[..host_len];
    if !FORGE_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return None;
    }
    let mut after = &from_host[host_len..];
    let mut consumed = host_start + host_off + host_len;
    if !scp && let Some(port) = after.strip_prefix(':') {
        let digits = byte_len(port.chars().take_while(char::is_ascii_digit));
        if digits > 0 && port[digits..].starts_with('/') {
            consumed += 1 + digits;
            after = &port[digits..];
        }
    }
    let sep = after.chars().next()?;
    if !(sep == '/' || (scp && sep == ':')) {
        return None;
    }
    consumed += 1;
    let path_raw = &after[1..];
    let path_len = path_raw.find(['?', '#']).unwrap_or(path_raw.len());
    let path = percent_decode(&path_raw[..path_len]);
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let owner = segments.next()?;
    let repo = segments.next().map(|r| r.trim_end_matches(".git"));
    let own = owner.eq_ignore_ascii_case(OWN_OWNER)
        && repo.is_none_or(|r| r.eq_ignore_ascii_case(OWN_REPO));
    if own {
        return None;
    }
    let placeholder = if repo.is_some() {
        "<org>/<repo>"
    } else {
        "<org>"
    };
    Some(format!("{}{placeholder}", &word[..consumed]))
}

/// Replace repository and organization identifiers on forge hosts with placeholders, except the
/// slopvac repository itself.
///
/// `https://github.com/acme/secret/issues/4` becomes `https://github.com/<org>/<repo>`; a bare
/// organization URL becomes `https://github.com/<org>`; the scp form `git@github.com:acme/x.git`
/// is handled the same way. Percent-escapes in the path are decoded before the check.
#[must_use]
pub fn scrub_repositories(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        // Sentence punctuation after a URL is not part of it.
        let trimmed = word.trim_end_matches(['.', ':', '!', '?']);
        let tail = &word[trimmed.len()..];
        match forge_word(trimmed) {
            Some(replaced) => {
                out.push_str(&replaced);
                out.push_str(tail);
            }
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in text.chars() {
        if is_repo_delimiter(c) {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Cleans report text for one machine and one working directory.
#[derive(Debug, Clone)]
pub struct Scrubber {
    /// Exact strings and what replaces them, longest first.
    literals: Vec<(String, &'static str)>,
    /// The fail-closed check that runs on the scrubbed text.
    gate: Gate,
}

impl Scrubber {
    /// Build a scrubber from the environment and the working directory.
    ///
    /// Names shorter than three characters are skipped, because replacing them would damage
    /// ordinary words.
    #[must_use]
    pub fn from_env(env: &dyn Env, cwd: &Path) -> Self {
        let mut literals: Vec<(String, &'static str)> = Vec::new();
        let mut add = |s: String, replacement: &'static str| {
            if s.len() >= 3 && !literals.iter().any(|(l, _)| *l == s) {
                literals.push((s, replacement));
            }
        };
        add(cwd.display().to_string(), "<path>");
        if let Ok(real) = std::fs::canonicalize(cwd) {
            add(real.display().to_string(), "<path>");
        }
        if let Some(home) = env.var("HOME") {
            add(home, "<path>");
        }
        for var in ["USER", "LOGNAME", "USERNAME"] {
            if let Some(user) = env.var(var) {
                add(user, "<user>");
            }
        }
        if let Some(host) = env.var("HOSTNAME") {
            add(host, "<host>");
        }
        #[cfg(unix)]
        add(
            rustix::system::uname()
                .nodename()
                .to_string_lossy()
                .into_owned(),
            "<host>",
        );
        literals.sort_by_key(|(l, _)| std::cmp::Reverse(l.len()));
        let locals = literals
            .iter()
            .filter(|(_, replacement)| *replacement != "<token>")
            .map(|(l, _)| l.clone())
            .collect();
        let gate = Gate::new(locals);
        Self { literals, gate }
    }

    /// The check for text as typed, before scrubbing: credential shapes only.
    #[must_use]
    pub fn check_raw(&self, text: &str) -> Option<Kind> {
        self.gate.check_raw(text)
    }

    /// The fail-closed check for text that has been scrubbed: what it still holds that looks
    /// like a credential or private data. The caller refuses the report; it never prints the text.
    #[must_use]
    pub fn check(&self, text: &str) -> Option<Kind> {
        self.gate.check(text)
    }

    fn literal(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (needle, replacement) in &self.literals {
            out = out.replace(needle.as_str(), replacement);
        }
        out
    }

    /// Clean a field the user typed: exact strings, path-like words and secrets.
    #[must_use]
    pub fn field(&self, text: &str) -> String {
        scrub_secrets(&scrub_paths(&scrub_repositories(&self.literal(text))))
    }

    /// Clean document text: exact strings and secrets only.
    #[must_use]
    pub fn sample(&self, text: &str) -> String {
        scrub_secrets(&scrub_repositories(&self.literal(text)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    /// Joins a token prefix and body at runtime, so the source never holds a token-shaped
    /// literal that a secret scanner would flag.
    fn fake(prefix: &str, body: &str) -> String {
        format!("{prefix}{body}")
    }

    #[test]
    fn path_like_words() {
        for (word, want) in [
            ("/home/alice/project/src/main.rs", true),
            ("/etc/hosts", true),
            ("/model", false),
            ("/", false),
            ("~/notes/todo.md", true),
            ("./run.sh", true),
            ("../up/one", true),
            ("docs/guide.md", true),
            ("notes.txt", true),
            ("README.md", true),
            ("sample.md", false),
            ("slopvac.toml", false),
            ("https://github.com/slopvac/slopvac/issues/1", false),
            ("and/or", false),
            ("3.0.0", false),
            ("--format", false),
            ("<stdin>", false),
        ] {
            assert_eq!(is_path_like(word), want, "{word}");
        }
    }

    #[test]
    fn windows_unc_and_extensionless_paths_are_paths() {
        for word in [
            r"C:\Users\bob\proj",
            "D:/work/x",
            r"\\fileserver\share\dir",
            r"relative\dir\file",
            "private/area/sub",
            "my-repo/src",
            "v1/api",
            "acme_corp/tool",
        ] {
            assert!(is_path_like(word), "{word}");
        }
        for word in [
            "and/or",
            "yes/no",
            "read/write",
            "input/output",
            "TCP/IP",
            "a:b",
            "C:",
            "x/",
        ] {
            assert!(!is_path_like(word), "{word}");
        }
    }

    #[test]
    fn single_backslash_names_are_paths_but_one_slash_between_words_is_prose() {
        for word in [r"acme\notes", r"work\private", r"a\b"] {
            assert!(is_path_like(word), "{word}");
        }
        for word in [r"\", r"trailing\", r"\leading", "and/or"] {
            assert!(!is_path_like(word) || word == r"\leading", "{word}");
        }
    }

    #[test]
    fn repositories_other_than_slopvac_are_replaced() {
        for (input, want) in [
            (
                "https://github.com/acme/secret/issues/4",
                "https://github.com/<org>/<repo>",
            ),
            ("https://github.com/acme", "https://github.com/<org>"),
            (
                "git@github.com:acme/secret.git",
                "git@github.com:<org>/<repo>",
            ),
            ("https://gitlab.com/g/h", "https://gitlab.com/<org>/<repo>"),
            (
                "https://github.com/slopvac/slopvac/issues/9",
                "https://github.com/slopvac/slopvac/issues/9",
            ),
            (
                "https://GitHub.com/acme/x",
                "https://GitHub.com/<org>/<repo>",
            ),
            (
                "https://github.com/slopvac/other",
                "https://github.com/<org>/<repo>",
            ),
            ("see github.com for more", "see github.com for more"),
        ] {
            assert_eq!(scrub_repositories(input), want, "{input}");
        }
    }

    #[test]
    fn bearer_with_extra_whitespace_is_still_redacted() {
        assert_eq!(scrub_secrets("Bearer    abc.def x"), "Bearer <token> x");
        assert_eq!(scrub_secrets("bearer\tabc.def x"), "Bearer <token> x");
        // Nothing after the word: nothing to redact, and the text is unchanged.
        assert_eq!(scrub_secrets("Bearer   "), "Bearer   ");
    }

    #[test]
    fn paths_become_placeholders_and_the_rest_stays() {
        let text =
            "ran `slopvac lint /home/alice/work/docs/a.md --format json` then opened notes.txt.";
        assert_eq!(
            scrub_paths(text),
            "ran `slopvac lint <path> --format json` then opened <path>."
        );
    }

    #[test]
    fn secrets_are_replaced() {
        let token = fake("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789");
        assert_eq!(
            scrub_secrets(&format!("use {token} here")),
            "use <token> here"
        );
        assert_eq!(
            scrub_secrets(&format!(
                "{} end",
                fake("github_pat_", "11AAAAAAA0abcdefghij_klmnopqrstuv")
            )),
            "<token> end"
        );
        assert_eq!(
            scrub_secrets("Authorization: Bearer abc.def-123 ok"),
            "Authorization: <redacted>"
        );
        assert_eq!(
            scrub_secrets("clone https://user:pw@example.com/x"),
            "clone https://example.com/x"
        );
        // A short run after a prefix is ordinary text.
        assert_eq!(scrub_secrets("ghp_ is a prefix"), "ghp_ is a prefix");
    }

    #[test]
    fn authorization_headers_lose_their_value_whatever_the_scheme() {
        for (input, want) in [
            (
                "Authorization: Basic dXNlcjpwYXNz",
                "Authorization: <redacted>",
            ),
            (
                "authorization:Token abc123SECRET",
                "authorization: <redacted>",
            ),
            (
                "Proxy-Authorization: Digest username=\"bob\"",
                "Proxy-Authorization: <redacted>",
            ),
            (
                "{\"Authorization\": \"Negotiate YII\"}",
                "{\"Authorization\": <redacted>",
            ),
            (
                "line1\nAuthorization: x\nline3",
                "line1\nAuthorization: <redacted>\nline3",
            ),
            ("the authorization process", "the authorization process"),
        ] {
            assert_eq!(scrub_secrets(input), want, "{input}");
        }
    }

    #[test]
    fn bare_schemes_with_a_credential_are_redacted_and_prose_is_not() {
        assert_eq!(
            scrub_secrets("sent Basic dXNlcjpwYXNzd29yZA== ok"),
            "sent Basic <token> ok"
        );
        assert_eq!(
            scrub_secrets("sent Token abc123SECRETvalue ok"),
            "sent Token <token> ok"
        );
        assert_eq!(scrub_secrets("Digest 0123456789abcdef"), "Digest <token>");
        for prose in [
            "Basic authentication works",
            "Token count is 5",
            "the token budget",
            "Digest of changes",
        ] {
            assert_eq!(scrub_secrets(prose), prose);
        }
    }

    #[test]
    fn a_github_token_split_by_whitespace_is_still_one_token() {
        for input in [
            fake("ghp_", "ABCDEFGH\nIJKLMNOPQRSTUVWXYZ0123456789"),
            fake("ghp_", "abcdef12 \n 3456789abcdefghijklmnopqrstu"),
            fake("ghp_", "ABCDEFGHIJKLMNOP  QRSTUVWXYZ0123456789"),
            format!(
                "{}\n{}",
                fake("github_pat_", "11AAAAAAA0abcdefghij"),
                "_klmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789012345678"
            ),
        ] {
            let out = scrub_secrets(&format!("x {input} y"));
            assert_eq!(out, "x <token> y", "{input}");
        }
        // Words after a short token stay.
        assert_eq!(
            scrub_secrets("ghp_abc12345 and then words"),
            "<token> and then words"
        );
    }

    #[test]
    fn percent_encoded_repository_paths_are_decoded_before_the_check() {
        for (input, want) in [
            (
                "https://github.com/marker%2Dorg/secret%2Drepo/issues/4",
                "https://github.com/<org>/<repo>",
            ),
            (
                "https://github.com/acme%2Fsecret",
                "https://github.com/<org>/<repo>",
            ),
            ("https://github.com/%61cme", "https://github.com/<org>"),
            (
                "https://github.com/slopvac%2Fslopvac/issues",
                "https://github.com/slopvac%2Fslopvac/issues",
            ),
        ] {
            assert_eq!(scrub_repositories(input), want, "{input}");
        }
    }

    #[test]
    fn forge_hosts_count_only_in_the_authority_or_scp_position() {
        for kept in [
            "https://example.com/github.com/guide",
            "https://docs.example.com/gitlab.com/acme/x",
            "see github.com for more",
            "https://github.com.evil.example/acme/x",
            "visit example.com?u=github.com/acme/x",
        ] {
            assert_eq!(scrub_repositories(kept), kept);
        }
        for (input, want) in [
            ("github.com/acme/secret", "github.com/<org>/<repo>"),
            (
                "https://user:pw@github.com:443/acme/secret?tab=x",
                "https://user:pw@github.com:443/<org>/<repo>",
            ),
            (
                "ssh://git@github.com/acme/secret.git",
                "ssh://git@github.com/<org>/<repo>",
            ),
            (
                "(see https://github.com/acme/secret.)",
                "(see https://github.com/<org>/<repo>.)",
            ),
        ] {
            assert_eq!(scrub_repositories(input), want, "{input}");
        }
    }

    #[test]
    fn exact_machine_strings_are_removed_everywhere() {
        let env = MapEnv::new(
            &[
                ("HOME", "/Users/zoe"),
                ("USER", "zoe-secret"),
                ("HOSTNAME", "box-77"),
            ],
            false,
        );
        let s = Scrubber::from_env(&env, Path::new("/work/marker-dir"));
        for text in [
            "cwd /work/marker-dir x",
            "home /Users/zoe y",
            "user zoe-secret z",
            "host box-77 w",
        ] {
            let field = s.field(text);
            let sample = s.sample(text);
            for leaked in ["marker-dir", "/Users/zoe", "zoe-secret", "box-77"] {
                assert!(!field.contains(leaked), "{text} -> {field}");
                assert!(!sample.contains(leaked), "{text} -> {sample}");
            }
        }
    }

    #[test]
    fn short_names_are_left_alone() {
        let env = MapEnv::new(&[("USER", "al")], false);
        let s = Scrubber::from_env(&env, Path::new("/x"));
        assert_eq!(s.field("also al here"), "also al here");
    }

    #[test]
    fn samples_keep_path_shaped_text() {
        let env = MapEnv::default();
        let s = Scrubber::from_env(&env, Path::new("/nonexistent-cwd-for-test"));
        assert_eq!(s.sample("see docs/guide.md"), "see docs/guide.md");
        assert_eq!(s.field("see docs/guide.md"), "see <path>");
    }
}
