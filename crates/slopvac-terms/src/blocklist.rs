//! Word blocklists in the 2.x `[[entries]]` format.
//!
//! A blocklist is opt-in and every entry needs a reason. Loading fails on a
//! missing `entries` array, an unknown part of speech, a missing reason or a
//! duplicate word and part of speech pair, so a typo cannot turn into a clean
//! report. TOML, JSON and YAML (`.yml`, `.yaml`) load.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::TermsError;

/// Parts of speech a blocklist entry can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pos {
    /// Noun.
    Noun,
    /// Verb.
    Verb,
    /// Adjective.
    Adjective,
    /// Adverb.
    Adverb,
    /// Preposition.
    Preposition,
    /// Conjunction.
    Conjunction,
    /// Pronoun.
    Pronoun,
    /// Article.
    Article,
}

impl Pos {
    /// Parse the abbreviations 2.x accepted, including spaCy-style tags.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let lower = raw.to_ascii_lowercase();
        Some(match lower.as_str() {
            "n" | "noun" => Self::Noun,
            "v" | "verb" => Self::Verb,
            "adj" | "adjective" => Self::Adjective,
            "adv" | "adverb" => Self::Adverb,
            "prep" | "preposition" | "adp" => Self::Preposition,
            "conj" | "conjunction" | "cconj" | "sconj" => Self::Conjunction,
            "pron" | "pronoun" => Self::Pronoun,
            "art" | "article" | "det" => Self::Article,
            _ => return None,
        })
    }
}

/// One refused word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlocklistEntry {
    /// The word, lower-cased and trimmed.
    pub word: String,
    /// The part of speech it is refused as.
    pub pos: Pos,
    /// Why it is refused.
    pub reason: String,
    /// Optional note on what to write instead.
    pub replacement: Option<String>,
}

/// A loaded blocklist, keyed by word and part of speech.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blocklist {
    entries: BTreeMap<(String, Pos), BlocklistEntry>,
}

#[derive(Deserialize)]
struct Raw {
    entries: Option<Vec<RawEntry>>,
    #[serde(flatten)]
    #[expect(
        clippy::zero_sized_map_values,
        reason = "only the key names are reported"
    )]
    other: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    word: Option<String>,
    pos: Option<String>,
    reason: Option<String>,
    replacement: Option<String>,
}

impl Blocklist {
    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the blocklist has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry for `word` as `pos`, if refused. The word match ignores case.
    #[must_use]
    pub fn lookup(&self, word: &str, pos: Pos) -> Option<&BlocklistEntry> {
        self.entries.get(&(word.to_lowercase(), pos))
    }

    /// Every entry, ordered by word then part of speech.
    pub fn iter(&self) -> impl Iterator<Item = &BlocklistEntry> {
        self.entries.values()
    }

    /// Read a blocklist file. The suffix picks the format: `.toml`, `.json`, `.yml` or `.yaml`.
    ///
    /// # Errors
    /// Unreadable or malformed files and invalid entries.
    pub fn load(path: &Path) -> Result<Self, TermsError> {
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|e| TermsError::Io {
            path: shown.clone(),
            message: e.to_string(),
        })?;
        let suffix = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        Self::parse(&shown, &suffix, &text)
    }

    /// Parse blocklist text of the given format (`toml`, `json`, `yml` or `yaml`).
    ///
    /// # Errors
    /// As [`Blocklist::load`].
    pub fn parse(path: &str, format: &str, text: &str) -> Result<Self, TermsError> {
        let parse_err = |message: String| TermsError::Parse {
            path: path.to_owned(),
            message,
        };
        let raw: Raw = match format {
            "toml" => toml::from_str(text).map_err(|e| parse_err(e.to_string()))?,
            "json" => serde_json::from_str(text).map_err(|e| parse_err(e.to_string()))?,
            // `no_schema` rejects unquoted numbers and booleans in string fields,
            // as TOML and JSON do.
            "yml" | "yaml" => {
                let mut options = serde_saphyr::Options::default();
                options.no_schema = true;
                // `n` and `v` are part-of-speech codes, not YAML 1.1 booleans.
                options.strict_booleans = true;
                serde_saphyr::from_str_with_options(text, options)
                    .map_err(|e| parse_err(e.to_string()))?
            }
            other => {
                return Err(parse_err(format!(
                    "unrecognized suffix `{other}`; use .toml, .json, .yml or .yaml"
                )));
            }
        };
        let Some(records) = raw.entries else {
            let named = raw
                .other
                .keys()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>();
            let named = if named.is_empty() {
                "nothing".to_owned()
            } else {
                named.join(", ")
            };
            return Err(parse_err(format!(
                "no `entries` array; it defines {named}. Each word is an `[[entries]]` table"
            )));
        };
        let entry_err = |message: String| TermsError::Entry {
            path: path.to_owned(),
            message,
        };
        let mut entries = BTreeMap::new();
        let mut at: BTreeMap<(String, Pos), usize> = BTreeMap::new();
        for (index, record) in records.into_iter().enumerate() {
            let number = index + 1;
            let word = record.word.unwrap_or_default().trim().to_lowercase();
            if word.is_empty() {
                return Err(entry_err(format!("entry {number} has no `word`")));
            }
            let raw_pos = record.pos.unwrap_or_default();
            let Some(pos) = Pos::parse(&raw_pos) else {
                return Err(entry_err(format!(
                    "entry `{word}` has an unrecognized `pos` {raw_pos:?}; use noun, verb, \
                     adjective, adverb, preposition, conjunction, pronoun or article"
                )));
            };
            let reason = record.reason.unwrap_or_default().trim().to_owned();
            if reason.is_empty() {
                return Err(entry_err(format!(
                    "entry `{word}` ({pos:?}) has no `reason`; every refused word needs one"
                )));
            }
            let key = (word.clone(), pos);
            if let Some(first) = at.get(&key) {
                return Err(entry_err(format!(
                    "duplicate entry `{word}` ({pos:?}) at entries {first} and {number}"
                )));
            }
            at.insert(key.clone(), number);
            entries.insert(
                key,
                BlocklistEntry {
                    word,
                    pos,
                    reason,
                    replacement: record
                        .replacement
                        .map(|r| r.trim().to_owned())
                        .filter(|r| !r.is_empty()),
                },
            );
        }
        Ok(Self { entries })
    }
}
