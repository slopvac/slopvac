//! The spelling locale: one generated rule, `ste-words.spelling`.
//!
//! Which spelling is correct depends on the project, so no shipped rule file holds this
//! rule. [`spelling_rules`] builds it for one locale, as 2.x does (`slopvac.locale`): a table
//! of variant words, each with its en-US and en-GB spelling, and the target locale decides
//! the direction. Every other locale's spelling of a word becomes a finding whose replacement
//! is the target's spelling, so one table serves both directions.
//!
//! 2.x has no spell checker. It has a hand-written variant table, regular inflections
//! generated from each base word, and an identifier allowlist. This module keeps those
//! semantics, so the data comes from `data/locale-variants.json`, which
//! `tests/fixtures/generate_locale.py` generates from the 2.x table. It carries no SCOWL or
//! `VarCon` data and no Hunspell dictionary.
//!
//! The locales are `en-US`, `en-GB` and `und`. `und` turns the check off without turning off
//! the rest of the category. Words the project spells its own way (`terms.allow`, and the
//! spellings of its terms) neither flag nor trigger a replacement.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, LoadErrors, RuleSet};
use slopvac_terms::TermSet;

/// The category the generated rule joins. It keeps that category's weight and budget.
pub const CATEGORY: &str = "ste-words";
/// The rule's id inside its category.
pub const RULE: &str = "spelling";

const DATA: &str = include_str!("../data/locale-variants.json");

#[derive(Deserialize)]
struct Example {
    bad: String,
    good: String,
}

#[derive(Deserialize)]
struct Data {
    locales: Vec<String>,
    default: String,
    variants: Vec<[String; 2]>,
    allowlist: Vec<String>,
    examples: HashMap<String, Vec<Example>>,
}

static TABLE: LazyLock<Data> =
    LazyLock::new(|| serde_json::from_str(DATA).expect("the bundled locale data is valid"));

/// A spelling locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    /// American English. The default.
    EnUs,
    /// British English.
    EnGb,
    /// Undetermined: the spelling check is off.
    Und,
}

impl Locale {
    /// The locale used when none is set.
    #[must_use]
    pub fn default_locale() -> Self {
        Self::parse(&TABLE.default).unwrap_or(Self::EnUs)
    }

    /// The locale with this tag, or `None` for a tag the table does not know.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        match tag {
            "en-US" => Some(Self::EnUs),
            "en-GB" => Some(Self::EnGb),
            "und" => Some(Self::Und),
            _ => None,
        }
    }

    /// The tags this build knows, in the order 2.x lists them.
    #[must_use]
    pub fn known() -> &'static [String] {
        &TABLE.locales
    }

    /// The tag, such as `en-GB`.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::EnGb => "en-GB",
            Self::Und => "und",
        }
    }

    /// Which side of a variant pair is this locale's spelling.
    const fn side(self) -> Option<usize> {
        match self {
            Self::EnUs => Some(0),
            Self::EnGb => Some(1),
            Self::Und => None,
        }
    }
}

/// Why the spelling rule could not be built.
#[derive(Debug)]
pub enum LocaleError {
    /// The tag is not a known locale. 2.x reported this as a note and skipped the check, so a
    /// typo did not stop the other rules.
    Unknown(String),
    /// The rules have no `ste-words` category to hold the rule.
    MissingCategory,
    /// The generated rule failed to load: a pattern did not compile, or an example
    /// disagreed with it.
    Load(LoadErrors),
}

impl fmt::Display for LocaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(tag) => write!(
                f,
                "locale \"{tag}\" is not known, so spelling was NOT checked. Known: {}.",
                Locale::known().join(", ")
            ),
            Self::MissingCategory => {
                write!(
                    f,
                    "the {CATEGORY} category is missing, so spelling was NOT checked."
                )
            }
            Self::Load(e) => write!(f, "the generated spelling rule failed to load: {e}"),
        }
    }
}

impl std::error::Error for LocaleError {}

/// The inflected forms of a base word. Only regular patterns, and only where the result is
/// unambiguous: a form this misses is a miss, and a form it invents is a false finding.
fn inflect(word: &str) -> Vec<String> {
    let mut forms = vec![word.to_owned()];
    let stem = &word[..word.len() - 1];
    if word.ends_with("ize")
        || word.ends_with("ise")
        || word.ends_with("yze")
        || word.ends_with("yse")
    {
        for suffix in ["es", "ed", "ing", "er", "ers"] {
            forms.push(format!("{stem}{suffix}"));
        }
    } else if [
        "or", "our", "er", "re", "og", "ogue", "ion", "ment", "ance", "ence",
    ]
    .iter()
    .any(|s| word.ends_with(s))
    {
        forms.push(format!("{word}s"));
    }
    forms
}

/// The substitutions and the allowlist of one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spelling {
    /// The locale the replacements spell in.
    pub locale: Locale,
    /// Other spelling to this locale's spelling, in the order 2.x lists them.
    pub substitutions: Vec<(String, String)>,
    /// Literal strings that never fire.
    pub allowlist: Vec<String>,
}

/// The spelling data for `locale` with `allow` words exempt, or `None` for `und`.
///
/// An allowed word is dropped from the substitutions, ignoring case, and joins the allowlist.
/// The built-in identifier allowlist keeps its place first. Words that repeat are listed once.
#[must_use]
pub fn spelling(locale: Locale, allow: &[String]) -> Option<Spelling> {
    let side = locale.side()?;
    let identifiers: BTreeSet<String> = TABLE.allowlist.iter().map(|w| w.to_lowercase()).collect();
    // A key set again keeps the position of its first insertion.
    let mut substitutions: Vec<(String, String)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for row in &TABLE.variants {
        let (target, other) = (&row[side], &row[1 - side]);
        if other == target {
            continue;
        }
        for (from, to) in inflect(other).into_iter().zip(inflect(target)) {
            if identifiers.contains(&from.to_lowercase())
                || from.to_lowercase() == to.to_lowercase()
            {
                continue;
            }
            if let Some(&at) = index.get(&from) {
                substitutions[at].1 = to;
            } else {
                index.insert(from.clone(), substitutions.len());
                substitutions.push((from, to));
            }
        }
    }
    let allowed: BTreeSet<String> = allow.iter().map(|w| w.to_lowercase()).collect();
    substitutions.retain(|(from, _)| !allowed.contains(&from.to_lowercase()));

    let mut allowlist: Vec<String> = Vec::new();
    for word in TABLE.allowlist.iter().chain(allow) {
        if !word.is_empty() && !allowlist.contains(word) {
            allowlist.push(word.clone());
        }
    }
    Some(Spelling {
        locale,
        substitutions,
        allowlist,
    })
}

/// The spellings of the terms in `terms`: each canonical spelling and variant. Terms the
/// project declares are product and language names, not prose, so they are exempt like
/// `terms.allow`.
#[must_use]
pub fn term_words(terms: &TermSet) -> Vec<String> {
    terms.spellings().map(str::to_owned).collect()
}

fn rule_json(spelling: &Spelling) -> Value {
    let tag = spelling.locale.tag();
    let examples: Vec<Value> = TABLE
        .examples
        .get(tag)
        .into_iter()
        .flatten()
        .map(|e| json!({"bad": e.bad, "good": e.good}))
        .collect();
    let substitutions: serde_json::Map<String, Value> = spelling
        .substitutions
        .iter()
        .map(|(from, to)| (from.clone(), Value::String(to.clone())))
        .collect();
    json!({
        "id": RULE,
        "name": format!("Use {tag} spelling"),
        "message": format!("Use the {tag} spelling \"{{replacement}}\"."),
        "severity": "warning",
        "scope": "prose",
        "match": {"kind": "substitution", "substitutions": substitutions, "ignore_case": true},
        "exceptions": [
            {"name": "identifier-fidelity"},
            {"name": "quotation", "detector": "quotation"},
            {"name": "code-span"},
            {"name": "proper-noun"},
            {"name": "api-name"},
        ],
        "allowlist": spelling.allowlist,
        "examples": examples,
        "fix": format!("Use the {tag} spelling."),
        "ai_signal": "none",
        "ai_signal_source": "unmeasured",
        "citation": {
            "source": "ASD-STE100",
            "ste_ref": "9:1.14",
            "note": "Generated from the locale table rather than written as a rule, because the \
                correct spelling depends on the project and a rule's substitution map has only \
                one direction. Rule 1.14 asks for American spelling; that is `locale = \"en-US\"`, \
                the default, rather than a rule a British English project cannot turn off.",
        },
    })
}

/// The generated spelling rule for `locale`, as a rule set holding one `ste-words` category.
///
/// The category copies the metadata of `base`'s `ste-words` category, so the rule scores with
/// that category's weight and budget. Pass the set to the engine beside `base`; its rule id is
/// `<pack>:ste-words.spelling`. A `spelling` rule that `base` already defines is replaced:
/// the engine gives a rule id to the last set that defines it, so pass this set after `base`.
///
/// Returns `Ok(None)` for `und`, which turns the check off on purpose.
///
/// # Errors
/// [`LocaleError::Unknown`] for a tag the table does not know, [`LocaleError::MissingCategory`]
/// when `base` has no `ste-words` category, and [`LocaleError::Load`] when the generated rule
/// does not load.
pub fn spelling_rules(
    pack: &PackId,
    base: &RuleSet,
    tag: &str,
    allow: &[String],
    options: &CompileOptions,
) -> Result<Option<RuleSet>, LocaleError> {
    let locale = Locale::parse(tag).ok_or_else(|| LocaleError::Unknown(tag.to_owned()))?;
    let Some(spelling) = spelling(locale, allow) else {
        return Ok(None);
    };
    let category = base
        .categories()
        .iter()
        .find(|c| c.id().as_str() == CATEGORY)
        .ok_or(LocaleError::MissingCategory)?;
    let mut doc = json!({
        "schema_version": 3,
        "id": CATEGORY,
        "title": category.title().as_str(),
        "description": category.description().as_str(),
        "weight": category.weight().get(),
        "rules": [rule_json(&spelling)],
    });
    if let Some(budget) = category.max_per_100_words() {
        doc["max_per_100_words"] = serde_json::to_value(budget).expect("a budget serializes");
    }
    if !category.recommended_for().is_empty() {
        doc["recommended_for"] =
            serde_json::to_value(category.recommended_for()).expect("genre ids serialize");
    }
    let file = PathBuf::from(format!("<generated locale {tag}>.json"));
    RuleSet::from_sources(pack, [(file, doc.to_string())], options)
        .map(Some)
        .map_err(LocaleError::Load)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(locale: Locale) -> Vec<(String, String)> {
        spelling(locale, &[]).unwrap().substitutions
    }

    #[test]
    fn und_has_no_spelling_check() {
        assert!(spelling(Locale::Und, &[]).is_none());
    }

    #[test]
    fn the_target_locale_is_the_replacement() {
        assert!(!pairs(Locale::EnGb).contains(&("colour".into(), "color".into())));
        assert!(pairs(Locale::EnUs).contains(&("colour".into(), "color".into())));
        assert!(pairs(Locale::EnGb).contains(&("normalize".into(), "normalise".into())));
    }

    #[test]
    fn inflections_follow_the_base_word() {
        let us = pairs(Locale::EnUs);
        assert!(us.contains(&("initialises".into(), "initializes".into())));
        assert!(us.contains(&("organisers".into(), "organizers".into())));
        assert!(us.contains(&("analysing".into(), "analyzing".into())));
    }

    #[test]
    fn identifier_words_are_never_substitutions() {
        // `color` is a CSS property name: American in every locale.
        assert!(!pairs(Locale::EnGb).iter().any(|(from, _)| from == "color"));
    }

    #[test]
    fn allowed_words_drop_out_ignoring_case() {
        let s = spelling(Locale::EnGb, &["Behavior".to_owned()]).unwrap();
        assert!(!s.substitutions.iter().any(|(from, _)| from == "behavior"));
        assert!(s.allowlist.contains(&"Behavior".to_owned()));
    }

    #[test]
    fn repeated_allowed_words_are_listed_once() {
        let s = spelling(Locale::EnGb, &["color".to_owned()]).unwrap();
        assert_eq!(s.allowlist.iter().filter(|w| *w == "color").count(), 1);
    }
}
