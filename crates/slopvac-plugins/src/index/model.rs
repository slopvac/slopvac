//! The index document: typed model, strict parser and canonical writer.
//!
//! The shape is `schemas/v3/pack-index.schema.json`; the semantics are in
//! `docs/contract/pack-index.md`.

use std::collections::HashSet;
use std::fmt;

use semver::Version;
use serde::Serialize;
use serde_json::{Map, Value};

use super::text::sanitize_for_terminal;

/// Largest index document the reader accepts, in bytes. Checked before parsing
/// (threat model N11).
pub const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
/// Most entries in one index.
pub const MAX_ENTRIES: usize = 100_000;
/// Longest pack id or namespace, in characters.
pub const MAX_ID_CHARS: usize = 64;
/// Longest display name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// Longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// Longest source, in characters.
pub const MAX_SOURCE_CHARS: usize = 2048;

/// A rejected index document or entry. `pointer` is a JSON Pointer into the
/// document (for example `/packs/0/source`); it is empty for the whole document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", if pointer.is_empty() { "/" } else { pointer })]
pub struct IndexError {
    /// JSON Pointer to the offending value.
    pub pointer: String,
    /// What is wrong. Unsafe characters from the document are escaped.
    pub message: String,
}

impl IndexError {
    pub(super) fn new(pointer: impl Into<String>, message: impl AsRef<str>) -> Self {
        Self {
            pointer: pointer.into(),
            message: sanitize_for_terminal(message.as_ref()),
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
    /// The spelling in the document.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Plugin => "plugin",
        }
    }
}

/// The key a publisher claims to sign with, as the index lists it.
///
/// **Display only** (threat model section 5.2). This type has no conversion
/// to a verification key or to a key policy, and none may be added: a key
/// becomes trusted only through the explicit pinning command, never through
/// the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherKey {
    /// Minisign key id: 16 uppercase hex digits.
    pub key_id: String,
    /// SHA-256 of the raw public key bytes: 64 lowercase hex digits.
    pub fingerprint: String,
}

impl fmt::Display for PublisherKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (fingerprint {})", self.key_id, self.fingerprint)
    }
}

/// One listed pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Pack id.
    pub id: String,
    /// Publisher namespace.
    pub namespace: String,
    /// Display name.
    pub name: String,
    /// One-line summary. Untrusted text: print it through
    /// [`sanitize_for_terminal`](super::sanitize_for_terminal).
    pub description: String,
    /// Latest listed version, without build metadata.
    pub version: Version,
    /// Data or plugin.
    pub kind: PackKind,
    /// Lowest extension API the host must implement.
    pub slopvac_api: u64,
    /// `oci://`, `https://` or pinned `git+https://` source.
    pub source: String,
    /// The publisher's claimed key. Display only.
    pub publisher_key: PublisherKey,
}

/// A validated index: entries sorted by namespace and id, with no pair repeated.
///
/// Canonical by construction, so `Index::parse(&index.to_json()) == Ok(index)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    packs: Vec<IndexEntry>,
}

fn is_id(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_hex(s: &str, len: usize, upper: bool) -> bool {
    s.len() == len
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || if upper {
                    (b'A'..=b'F').contains(&b)
                } else {
                    (b'a'..=b'f').contains(&b)
                }
        })
}

/// True when the text after `scheme://` has a non-empty authority without
/// user-info and no whitespace anywhere.
fn is_url_rest(rest: &str) -> bool {
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    !authority.is_empty() && !authority.contains('@') && !rest.chars().any(char::is_whitespace)
}

// NOTE: the same source forms as the lock reader (packs/9.3 lock/model.rs),
// minus path sources. Fold the two into one type when that module is on main.
fn is_source(s: &str) -> bool {
    if let Some(rest) = s
        .strip_prefix("oci://")
        .or_else(|| s.strip_prefix("https://"))
    {
        return is_url_rest(rest);
    }
    s.strip_prefix("git+https://")
        .and_then(|rest| rest.rsplit_once('@'))
        .is_some_and(|(url, commit)| is_url_rest(url) && is_hex(commit, 40, false))
}

fn check_len(value: &str, min: usize, max: usize, at: &str, what: &str) -> Result<(), IndexError> {
    let n = value.chars().count();
    if (min..=max).contains(&n) {
        Ok(())
    } else {
        Err(IndexError::new(
            at,
            format!("{what} must have {min} to {max} characters, not {n}"),
        ))
    }
}

impl IndexEntry {
    fn validate(&self, at: &str) -> Result<(), IndexError> {
        let p = |key: &str| format!("{at}/{key}");
        for (key, value) in [("id", &self.id), ("namespace", &self.namespace)] {
            check_len(value, 1, MAX_ID_CHARS, &p(key), key)?;
            if !is_id(value) {
                return Err(IndexError::new(
                    p(key),
                    format!(
                        "`{value}` is not a valid {key} (lowercase letters, digits and `-`, starting with a letter)"
                    ),
                ));
            }
        }
        check_len(&self.name, 1, MAX_NAME_CHARS, &p("name"), "name")?;
        check_len(
            &self.description,
            1,
            MAX_DESCRIPTION_CHARS,
            &p("description"),
            "description",
        )?;
        if !self.version.build.is_empty() {
            return Err(IndexError::new(
                p("version"),
                "a version must not carry build metadata",
            ));
        }
        if self.slopvac_api == 0 {
            return Err(IndexError::new(p("slopvac_api"), "slopvac_api starts at 1"));
        }
        check_len(&self.source, 1, MAX_SOURCE_CHARS, &p("source"), "source")?;
        if !is_source(&self.source) {
            return Err(IndexError::new(
                p("source"),
                format!(
                    "`{}` is not an oci://, https:// or pinned git+https:// source without user-info",
                    self.source
                ),
            ));
        }
        if !is_hex(&self.publisher_key.key_id, 16, true) {
            return Err(IndexError::new(
                p("publisher_key/key_id"),
                "a key id is 16 uppercase hex digits",
            ));
        }
        if !is_hex(&self.publisher_key.fingerprint, 64, false) {
            return Err(IndexError::new(
                p("publisher_key/fingerprint"),
                "a fingerprint is 64 lowercase hex digits",
            ));
        }
        Ok(())
    }
}

impl Index {
    /// Validates every entry, rejects a repeated namespace and id pair, and
    /// sorts by namespace and then id. Error pointers use the positions in
    /// `packs` as given.
    pub fn new(mut packs: Vec<IndexEntry>) -> Result<Self, IndexError> {
        if packs.len() > MAX_ENTRIES {
            return Err(IndexError::new(
                "/packs",
                format!("more than {MAX_ENTRIES} entries"),
            ));
        }
        let mut seen = HashSet::with_capacity(packs.len());
        for (i, entry) in packs.iter().enumerate() {
            entry.validate(&format!("/packs/{i}"))?;
            if !seen.insert((&entry.namespace, &entry.id)) {
                return Err(IndexError::new(
                    format!("/packs/{i}/id"),
                    format!("pack `{}/{}` is listed twice", entry.namespace, entry.id),
                ));
            }
        }
        packs.sort_by(|a, b| (&a.namespace, &a.id).cmp(&(&b.namespace, &b.id)));
        Ok(Self { packs })
    }

    /// The entries, sorted by namespace and id.
    #[must_use]
    pub fn entries(&self) -> &[IndexEntry] {
        &self.packs
    }

    /// Parses an index document. The size limit is checked before any parsing.
    /// Unknown keys are errors and `x-` keys are ignored.
    pub fn parse(bytes: &[u8]) -> Result<Self, IndexError> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(IndexError::new(
                "",
                format!(
                    "the index is {} bytes; the limit is {MAX_INDEX_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| IndexError::new("", format!("not valid JSON: {e}")))?;
        let root = object(&value, "")?;
        only_keys(root, &["schema_version", "packs"], "")?;
        if root.get("schema_version").and_then(Value::as_u64) != Some(3) {
            return Err(IndexError::new(
                "/schema_version",
                "schema_version must be 3",
            ));
        }
        let list = root
            .get("packs")
            .and_then(Value::as_array)
            .ok_or_else(|| IndexError::new("/packs", "packs must be an array"))?;
        if list.len() > MAX_ENTRIES {
            return Err(IndexError::new(
                "/packs",
                format!("more than {MAX_ENTRIES} entries"),
            ));
        }
        let packs = list
            .iter()
            .enumerate()
            .map(|(i, v)| parse_entry(v, &format!("/packs/{i}")))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(packs)
    }

    /// Writes the canonical document: fixed key order, two-space indent, a
    /// final newline. The same index always gives the same bytes.
    #[must_use]
    pub fn to_json(&self) -> String {
        let out = IndexOut {
            schema_version: 3,
            packs: self.packs.iter().map(EntryOut::from).collect(),
        };
        let mut text = serde_json::to_string_pretty(&out).unwrap_or_default();
        text.push('\n');
        text
    }
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, IndexError> {
    value
        .as_object()
        .ok_or_else(|| IndexError::new(at, "expected an object"))
}

fn only_keys(map: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), IndexError> {
    match map
        .keys()
        .find(|k| !allowed.contains(&k.as_str()) && !k.starts_with("x-"))
    {
        Some(key) => Err(IndexError::new(at, format!("unknown key `{key}`"))),
        None => Ok(()),
    }
}

fn string<'a>(map: &'a Map<String, Value>, key: &str, at: &str) -> Result<&'a str, IndexError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| IndexError::new(format!("{at}/{key}"), format!("`{key}` must be a string")))
}

fn parse_entry(value: &Value, at: &str) -> Result<IndexEntry, IndexError> {
    let map = object(value, at)?;
    only_keys(
        map,
        &[
            "id",
            "namespace",
            "name",
            "description",
            "version",
            "kind",
            "slopvac_api",
            "source",
            "publisher_key",
        ],
        at,
    )?;
    let version_text = string(map, "version", at)?;
    let version = Version::parse(version_text)
        .map_err(|e| IndexError::new(format!("{at}/version"), format!("`{version_text}`: {e}")))?;
    let kind = match string(map, "kind", at)? {
        "data" => PackKind::Data,
        "plugin" => PackKind::Plugin,
        other => {
            return Err(IndexError::new(
                format!("{at}/kind"),
                format!("`{other}` is not a pack kind (`data` or `plugin`)"),
            ));
        }
    };
    let slopvac_api = map
        .get("slopvac_api")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            IndexError::new(
                format!("{at}/slopvac_api"),
                "slopvac_api must be a positive integer",
            )
        })?;
    let key_at = format!("{at}/publisher_key");
    let key_map = map
        .get("publisher_key")
        .ok_or_else(|| IndexError::new(&key_at, "publisher_key is required"))
        .and_then(|v| object(v, &key_at))?;
    only_keys(key_map, &["key_id", "fingerprint"], &key_at)?;
    Ok(IndexEntry {
        id: string(map, "id", at)?.to_owned(),
        namespace: string(map, "namespace", at)?.to_owned(),
        name: string(map, "name", at)?.to_owned(),
        description: string(map, "description", at)?.to_owned(),
        version,
        kind,
        slopvac_api,
        source: string(map, "source", at)?.to_owned(),
        publisher_key: PublisherKey {
            key_id: string(key_map, "key_id", &key_at)?.to_owned(),
            fingerprint: string(key_map, "fingerprint", &key_at)?.to_owned(),
        },
    })
}

#[derive(Serialize)]
struct IndexOut<'a> {
    schema_version: u8,
    packs: Vec<EntryOut<'a>>,
}

#[derive(Serialize)]
struct EntryOut<'a> {
    id: &'a str,
    namespace: &'a str,
    name: &'a str,
    description: &'a str,
    version: String,
    kind: &'static str,
    slopvac_api: u64,
    source: &'a str,
    publisher_key: KeyOut<'a>,
}

#[derive(Serialize)]
struct KeyOut<'a> {
    key_id: &'a str,
    fingerprint: &'a str,
}

impl<'a> From<&'a IndexEntry> for EntryOut<'a> {
    fn from(e: &'a IndexEntry) -> Self {
        Self {
            id: &e.id,
            namespace: &e.namespace,
            name: &e.name,
            description: &e.description,
            version: e.version.to_string(),
            kind: e.kind.as_str(),
            slopvac_api: e.slopvac_api,
            source: &e.source,
            publisher_key: KeyOut {
                key_id: &e.publisher_key.key_id,
                fingerprint: &e.publisher_key.fingerprint,
            },
        }
    }
}
