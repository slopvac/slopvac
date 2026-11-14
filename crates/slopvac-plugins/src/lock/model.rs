use std::fmt;
use std::str::FromStr;

use semver::Version;

/// A rejected value, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValueError(String);

fn bad<T>(message: impl Into<String>) -> Result<T, ValueError> {
    Err(ValueError(message.into()))
}

/// A lock file error. `pointer` is a JSON Pointer into the file
/// (for example `/packs/0/digest`); it is empty for a whole-file error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", if pointer.is_empty() { "/" } else { pointer })]
pub struct LockError {
    /// JSON Pointer to the offending value.
    pub pointer: String,
    /// What is wrong.
    pub message: String,
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
            ValueError(format!(
                "`{s}` is not a UTC time (`YYYY-MM-DDTHH:MM:SSZ`, calendar-valid)"
            ))
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

/// Where a locked pack came from. Display gives the string stored in the file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
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

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oci(s) | Self::Archive(s) | Self::Path(s) => f.write_str(s),
            Self::Git { url, commit } => write!(f, "git+{url}@{commit}"),
        }
    }
}

/// Mirrors `[A-Za-z][A-Za-z0-9+.-]*:` at the start of the string.
fn has_scheme_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))) == Some(':')
}

impl FromStr for Source {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, ValueError> {
        let no_space = |rest: &str| !rest.is_empty() && !rest.chars().any(char::is_whitespace);
        if let Some(rest) = s.strip_prefix("oci://") {
            return if no_space(rest) {
                Ok(Self::Oci(s.to_owned()))
            } else {
                bad("an oci:// source needs a reference without whitespace")
            };
        }
        if let Some(rest) = s.strip_prefix("https://") {
            return if no_space(rest) {
                Ok(Self::Archive(s.to_owned()))
            } else {
                bad("an https:// source needs a URL without whitespace")
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
                Some((url, commit)) => Ok(Self::Git {
                    url: format!("https://{url}"),
                    commit: commit.to_owned(),
                }),
                None => bad(
                    "a git+https:// source must end in `@` and a full 40-digit lowercase commit hash",
                ),
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
        Ok(Self::Path(s.to_owned()))
    }
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
    /// The extension API the pack targets (at least 1).
    pub slopvac_api: u32,
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
    /// Builds a lock file from entries. Sorts them by id and sorts each
    /// `requires`. Fails on a repeated pack id.
    pub fn new(mut packs: Vec<LockedPack>) -> Result<Self, LockError> {
        packs.sort_by(|a, b| a.id.cmp(&b.id));
        for pack in &mut packs {
            pack.requires.sort();
            pack.requires.dedup();
        }
        if let Some(i) = packs.windows(2).position(|w| w[0].id == w[1].id) {
            return Err(LockError {
                pointer: format!("/packs/{}/id", i + 1),
                message: format!("pack id `{}` is locked twice", packs[i].id),
            });
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
