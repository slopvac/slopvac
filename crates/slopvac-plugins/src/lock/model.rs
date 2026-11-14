use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::str::FromStr;

use semver::Version;

/// Largest lock file the reader accepts, in bytes. Checked before parsing.
pub const MAX_LOCK_BYTES: usize = 1024 * 1024;
/// Most packs in one lock file.
pub const MAX_PACKS: usize = 1000;
/// Most `requires` entries in one pack.
pub const MAX_REQUIRES: usize = 256;
/// Longest string value, in bytes.
pub const MAX_STRING_BYTES: usize = 4096;

/// A rejected value, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValueError(String);

/// True for characters that can forge or hide terminal output: C0, DEL and C1
/// controls, bidirectional controls, zero-width and line/paragraph separators.
fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'
        )
}

/// Replaces unsafe characters (and newlines) with `\u{..}` so that text from a
/// lock file cannot forge or hide output when it is printed.
pub(super) fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_unsafe(c) {
            let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

fn bad<T>(message: impl AsRef<str>) -> Result<T, ValueError> {
    Err(ValueError(sanitize(message.as_ref())))
}

/// A lock file error. `pointer` is a JSON Pointer into the file
/// (for example `/packs/0/digest`); it is empty for a whole-file error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", if pointer.is_empty() { "/" } else { pointer })]
pub struct LockError {
    /// JSON Pointer to the offending value.
    pub pointer: String,
    /// What is wrong. Unsafe characters from the file are escaped.
    pub message: String,
}

impl LockError {
    pub(super) fn new(pointer: impl Into<String>, message: impl AsRef<str>) -> Self {
        Self {
            pointer: pointer.into(),
            message: sanitize(message.as_ref()),
        }
    }
}

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The value as written in the file.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_newtype!(
    /// A pack id: `^[a-z][a-z0-9-]*$`.
    PackId
);
string_newtype!(
    /// A content digest: `sha256:` plus 64 lowercase hex digits.
    Digest
);
string_newtype!(
    /// A minisign key id: 16 uppercase hex digits.
    KeyId
);
string_newtype!(
    /// A UTC `notBefore` time, `YYYY-MM-DDTHH:MM:SSZ`. Fixed width, so string
    /// order is chronological order.
    NotBefore
);

impl FromStr for PackId {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        let mut chars = s.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            bad(format!(
                "`{s}` is not a pack id (lowercase letters, digits and `-`, starting with a letter)"
            ))
        }
    }
}

impl FromStr for Digest {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        let ok = s.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            bad(format!(
                "`{s}` is not a digest (`sha256:` and 64 lowercase hex digits)"
            ))
        }
    }
}

impl FromStr for KeyId {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        if s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'F')) {
            Ok(Self(s.to_owned()))
        } else {
            bad(format!("`{s}` is not a key id (16 uppercase hex digits)"))
        }
    }
}

impl FromStr for NotBefore {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        let format_error = || {
            ValueError(sanitize(&format!(
                "`{s}` is not a UTC time (`YYYY-MM-DDTHH:MM:SSZ`, calendar-valid)"
            )))
        };
        let b = s.as_bytes();
        if b.len() != 20 {
            return Err(format_error());
        }
        let fixed = [
            (4, b'-'),
            (7, b'-'),
            (10, b'T'),
            (13, b':'),
            (16, b':'),
            (19, b'Z'),
        ];
        let separators_ok = fixed.iter().all(|&(i, c)| b[i] == c);
        let digits_ok = b
            .iter()
            .enumerate()
            .all(|(i, c)| fixed.iter().any(|&(j, _)| i == j) || c.is_ascii_digit());
        if !(separators_ok && digits_ok) {
            return Err(format_error());
        }
        let num = |range: std::ops::Range<usize>| s[range].parse::<u32>().unwrap_or(u32::MAX);
        let (year, month, day) = (num(0..4), num(5..7), num(8..10));
        let (hour, minute, second) = (num(11..13), num(14..16), num(17..19));
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return Err(format_error()),
        };
        if (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60 {
            Ok(Self(s.to_owned()))
        } else {
            Err(format_error())
        }
    }
}

/// What a pack is: data only, or data plus a plugin command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackKind {
    /// Rules, styles, genres, terms and taggers. No code.
    Data,
    /// Adds a command for the host to run.
    Plugin,
}

impl PackKind {
    /// The spelling in the file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Plugin => "plugin",
        }
    }
}

impl FromStr for PackKind {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        match s {
            "data" => Ok(Self::Data),
            "plugin" => Ok(Self::Plugin),
            _ => bad(format!("`{s}` is not a pack kind (`data` or `plugin`)")),
        }
    }
}

/// The shape of a [`Source`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// An `oci://` reference.
    Oci(String),
    /// An `https://` release archive URL.
    Archive(String),
    /// A `git+https://` repository pinned to a full commit.
    Git {
        /// The `https://` repository URL.
        url: String,
        /// The 40-digit lowercase commit hash.
        commit: String,
    },
    /// A path relative to the project root, with forward slashes.
    Path(String),
}

/// Where a locked pack came from. Built only by parsing, so every value obeys
/// the contract: no `http://`, no user-info, git pinned to a commit, paths
/// relative. Display gives the string stored in the file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Source {
    kind: SourceKind,
}

impl Source {
    /// The parsed shape.
    #[must_use]
    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            SourceKind::Oci(s) | SourceKind::Archive(s) | SourceKind::Path(s) => f.write_str(s),
            SourceKind::Git { url, commit } => write!(f, "git+{url}@{commit}"),
        }
    }
}

/// Mirrors `[A-Za-z][A-Za-z0-9+.-]*:` at the start of the string.
fn has_scheme_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))) == Some(':')
}

/// True when the URL authority (the text after `://` up to the first `/`,
/// `?` or `#`) holds user-info. A lock is committed to the repository, so it
/// never carries credentials.
fn has_user_info(after_scheme: &str) -> bool {
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    after_scheme[..end].contains('@')
}

impl FromStr for Source {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        parse_kind(s).map(|kind| Self { kind })
    }
}

fn parse_kind(s: &str) -> Result<SourceKind, ValueError> {
    let no_space = |rest: &str| !rest.is_empty() && !rest.chars().any(char::is_whitespace);
    let no_credentials = || bad("a source URL must not carry user-info (credentials)");
    if let Some(rest) = s.strip_prefix("oci://") {
        return match (no_space(rest), has_user_info(rest)) {
            (false, _) => bad("an oci:// source needs a reference without whitespace"),
            (_, true) => no_credentials(),
            _ => Ok(SourceKind::Oci(s.to_owned())),
        };
    }
    if let Some(rest) = s.strip_prefix("https://") {
        return match (no_space(rest), has_user_info(rest)) {
            (false, _) => bad("an https:// source needs a URL without whitespace"),
            (_, true) => no_credentials(),
            _ => Ok(SourceKind::Archive(s.to_owned())),
        };
    }
    if let Some(rest) = s.strip_prefix("git+https://") {
        let pinned = rest.rsplit_once('@').filter(|(url, commit)| {
            no_space(url)
                && commit.len() == 40
                && commit
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        return match pinned {
            None => bad(
                "a git+https:// source must end in `@` and a full 40-digit lowercase commit hash",
            ),
            Some((url, _)) if has_user_info(url) => no_credentials(),
            Some((url, commit)) => Ok(SourceKind::Git {
                url: format!("https://{url}"),
                commit: commit.to_owned(),
            }),
        };
    }
    if s.starts_with("http://") {
        return bad("http:// sources are not allowed; use https://");
    }
    if has_scheme_prefix(s) {
        return bad("the source scheme is not oci, https or git+https");
    }
    if s.starts_with('/') || s.contains('\\') {
        return bad("a path source must be relative and use forward slashes");
    }
    if s.split('/').any(|segment| segment == "..") {
        return bad("a path source must not contain a `..` segment");
    }
    if s.chars().next().is_none_or(char::is_whitespace) || s.contains(['\n', '\r']) {
        return bad("a path source must start with a non-space character and fit on one line");
    }
    Ok(SourceKind::Path(s.to_owned()))
}

/// A verified minisign signature, as recorded by `plugins sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The signing key.
    pub key_id: KeyId,
    /// The `notBefore` time of the signed statement; the rollback floor.
    pub not_before: NotBefore,
}

/// One locked pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPack {
    /// Pack id.
    pub id: PackId,
    /// Data or plugin.
    pub kind: PackKind,
    /// Semantic version without build metadata.
    pub version: Version,
    /// The extension API the pack targets (at least 1, no upper bound).
    pub slopvac_api: u64,
    /// Where the bytes came from.
    pub source: Source,
    /// Content digest of the pack.
    pub digest: Digest,
    /// Present when the host verified a signature.
    pub signature: Option<Signature>,
    /// Ids of locked packs this one needs, sorted and unique.
    pub requires: Vec<PackId>,
}

/// A parsed `.slopvac/packs.lock`.
///
/// `packs` is sorted by id and ids are unique, so the model is already in
/// canonical form: `Lockfile::parse(&lock.to_toml()) == Ok(lock)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lockfile {
    packs: Vec<LockedPack>,
}

impl Lockfile {
    /// Builds a lock file from entries and checks every invariant the reader
    /// enforces, so [`Lockfile::to_toml`] always writes text that
    /// [`Lockfile::parse`] accepts. Sorts the entries by id and each `requires`.
    /// Error pointers use the positions in `packs` as given.
    ///
    /// `requires` may name any valid pack id, locked or not: `core` is the
    /// built-in pack and has no entry (`packs.md`, section 2).
    pub fn new(mut packs: Vec<LockedPack>) -> Result<Self, LockError> {
        if packs.len() > MAX_PACKS {
            return Err(LockError::new(
                "/packs",
                format!("more than {MAX_PACKS} packs"),
            ));
        }
        let mut seen = HashSet::with_capacity(packs.len());
        for (i, pack) in packs.iter().enumerate() {
            let at = |key: &str| format!("/packs/{i}/{key}");
            if !seen.insert(&pack.id) {
                return Err(LockError::new(
                    at("id"),
                    format!("pack id `{}` is locked twice", pack.id),
                ));
            }
            if !pack.version.build.is_empty() {
                return Err(LockError::new(
                    at("version"),
                    format!("`{}` has build metadata", pack.version),
                ));
            }
            if pack.slopvac_api < 1 {
                return Err(LockError::new(at("slopvac_api"), "must be at least 1"));
            }
            if pack.requires.len() > MAX_REQUIRES {
                return Err(LockError::new(
                    at("requires"),
                    format!("more than {MAX_REQUIRES} entries"),
                ));
            }
            let mut ids = HashSet::with_capacity(pack.requires.len());
            if let Some(j) = pack.requires.iter().position(|r| !ids.insert(r)) {
                return Err(LockError::new(
                    format!("{}/{j}", at("requires")),
                    format!("`{}` is listed twice", pack.requires[j]),
                ));
            }
        }
        packs.sort_by(|a, b| a.id.cmp(&b.id));
        for pack in &mut packs {
            pack.requires.sort();
        }
        Ok(Self { packs })
    }

    /// The entries, sorted by id.
    #[must_use]
    pub fn packs(&self) -> &[LockedPack] {
        &self.packs
    }

    /// The entry for `id`.
    #[must_use]
    pub fn get(&self, id: &PackId) -> Option<&LockedPack> {
        self.packs
            .binary_search_by(|p| p.id.cmp(id))
            .ok()
            .map(|i| &self.packs[i])
    }

    /// Reads `.slopvac/packs.lock` text. Unknown keys are errors and `x-` keys
    /// are ignored.
    pub fn parse(text: &str) -> Result<Self, LockError> {
        super::parse::parse(text)
    }

    /// The canonical text: stable key order, entries sorted by id, LF endings.
    #[must_use]
    pub fn to_toml(&self) -> String {
        super::write::write(self)
    }
}
