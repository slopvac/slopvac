//! Word blocklists in the `[[entries]]` word, part of speech and reason format.
//!
//! A blocklist is opt-in and every entry needs a reason. Loading fails on a
//! missing or wrong `schema_version` (a 2.x blocklist has none), a missing
//! `entries` array, an unknown key that does not start with `x-`, an unknown part of speech, a missing reason or a
//! duplicate word and part of speech pair, so a typo cannot turn into a clean
//! report. TOML, JSON and YAML (`.yml`, `.yaml`) load.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::{EntryProblem, TermsError};

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
    /// Parse a part of speech exactly as the blocklist schema lists it.
    ///
    /// The match is case-sensitive and does not trim: the lowercase short and
    /// long forms, and the uppercase spaCy tags.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "n" | "noun" | "NOUN" => Self::Noun,
            "v" | "verb" | "VERB" => Self::Verb,
            "adj" | "adjective" | "ADJ" => Self::Adjective,
            "adv" | "adverb" | "ADV" => Self::Adverb,
            "prep" | "preposition" | "ADP" => Self::Preposition,
            "conj" | "conjunction" | "CCONJ" | "SCONJ" => Self::Conjunction,
            "pron" | "pronoun" | "PRON" => Self::Pronoun,
            "art" | "article" | "DET" => Self::Article,
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
    // Kept as a value so a quoted `"3"` stays a string instead of coercing to 3.
    schema_version: Option<serde_json::Value>,
    entries: Option<Vec<RawEntry>>,
    #[serde(flatten)]
    #[allow(clippy::zero_sized_map_values)] // only the key names are reported
    other: BTreeMap<String, serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct RawEntry {
    word: Option<String>,
    pos: Option<String>,
    reason: Option<String>,
    replacement: Option<String>,
    #[serde(flatten)]
    #[allow(clippy::zero_sized_map_values)] // only the key names are checked
    other: BTreeMap<String, serde::de::IgnoredAny>,
}

/// Require `schema_version = 3`. A 2.x blocklist has none.
fn check_version(path: &str, declared: Option<&serde_json::Value>) -> Result<(), TermsError> {
    match declared {
        Some(found) if found.as_u64() == Some(3) => Ok(()),
        Some(found) => Err(TermsError::SchemaVersion {
            path: path.to_owned(),
            found: found.to_string(),
        }),
        None => Err(TermsError::MissingSchemaVersion {
            path: path.to_owned(),
        }),
    }
}

/// Read the text of a blocklist file into its raw records.
fn read_raw(path: &str, format: &str, text: &str) -> Result<Raw, TermsError> {
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
    Ok(raw)
}

impl Blocklist {
    /// Add the entries of `other` that this blocklist does not have. An entry for the same word
    /// and part of speech that is already here wins, so list the most specific file first.
    pub fn merge(&mut self, other: Self) {
        for (key, entry) in other.entries {
            self.entries.entry(key).or_insert(entry);
        }
    }

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
        Self::load_labeled(path, &path.display().to_string())
    }

    /// Like [`Blocklist::load`], naming the file `shown` in errors. Pass a label that is
    /// relative to the project root, so an error never carries an absolute path.
    ///
    /// # Errors
    /// As [`Blocklist::load`].
    pub fn load_labeled(path: &Path, shown: &str) -> Result<Self, TermsError> {
        let shown = shown.to_owned();
        let text = crate::read::read_existing(path, &shown, crate::MAX_TERMS_FILE_BYTES)?;
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
        let raw = read_raw(path, format, text)?;
        check_version(path, raw.schema_version.as_ref())?;
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
            return Err(TermsError::Parse {
                path: path.to_owned(),
                message: format!(
                    "no `entries` array; it defines {named}. Each word is an `[[entries]]` table"
                ),
            });
        };
        let unknown = |key: String| TermsError::UnknownKey {
            path: path.to_owned(),
            key,
        };
        if let Some(key) = raw.other.keys().find(|k| !k.starts_with("x-")) {
            return Err(unknown(key.clone()));
        }
        let entry_err = |problem: EntryProblem, number: usize, message: String| TermsError::Entry {
            path: path.to_owned(),
            message,
            number: Some(number),
            problem,
        };
        let mut entries = BTreeMap::new();
        let mut at: BTreeMap<(String, Pos), usize> = BTreeMap::new();
        for (index, record) in records.into_iter().enumerate() {
            let number = index + 1;
            if let Some(key) = record.other.keys().find(|k| !k.starts_with("x-")) {
                return Err(unknown(format!("entries[{number}].{key}")));
            }
            let word = record.word.unwrap_or_default().trim().to_lowercase();
            if word.is_empty() {
                return Err(entry_err(
                    EntryProblem::MissingWord,
                    number,
                    format!("entry {number} has no `word`"),
                ));
            }
            let raw_pos = record.pos.unwrap_or_default();
            let Some(pos) = Pos::parse(&raw_pos) else {
                return Err(entry_err(
                    EntryProblem::UnknownPos,
                    number,
                    format!(
                        "entry `{word}` has an unrecognized `pos` {raw_pos:?}; use noun, verb, \
                         adjective, adverb, preposition, conjunction, pronoun or article"
                    ),
                ));
            };
            let reason = record.reason.unwrap_or_default().trim().to_owned();
            if reason.is_empty() {
                return Err(entry_err(
                    EntryProblem::MissingReason,
                    number,
                    format!(
                        "entry `{word}` ({pos:?}) has no `reason`; every refused word needs one"
                    ),
                ));
            }
            let key = (word.clone(), pos);
            if let Some(first) = at.get(&key) {
                return Err(entry_err(
                    EntryProblem::Duplicate,
                    number,
                    format!("duplicate entry `{word}` ({pos:?}) at entries {first} and {number}"),
                ));
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
