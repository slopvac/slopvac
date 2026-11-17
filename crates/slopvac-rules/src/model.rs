//! The typed v3 rule and category model. It mirrors `rule.schema.json` and
//! `category.schema.json`; `docs/contract/rules.md` states the semantics.
//!
//! `Deserialize` rejects what the schemas reject: unknown keys are errors, `x-` keys are
//! ignored, and the cross-field constraints hold. The generated schemas come from
//! [`crate::rule_file_schema`] and [`crate::category_schema`].

// `PlaceholderSource` has one variant today and gains more when a second source appears.
#![allow(clippy::zero_sized_map_values)]

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use slopvac_model::schema::{SLUG, if_then, x_extensions};
use slopvac_model::{
    AiSignal, CategoryId, ContextTag, Density, GenreId, NonEmptyString, SchemaVersion, Severity,
};

use crate::ext::{Extensions, UniqueVec, ValueError, is_slug, present, text_newtype};
use crate::schema::Kind;

text_newtype!(
    /// Rule id inside its category: a lower-case letter, then lower-case letters, digits and
    /// hyphens.
    LocalRuleId, false,
    "Rule id inside its category. The qualified RuleId is <pack>:<category>.<id>, built from \
     the pack manifest, the category id and this id.",
    "rule id", |s| is_slug(s, '-'), SLUG
);
text_newtype!(
    /// A name the engine resolves: a structure or vocabulary check, an exception or a
    /// detector. A lower-case letter, then lower-case letters, digits and hyphens.
    Slug, true, "", "name", |s| is_slug(s, '-'), SLUG
);
text_newtype!(
    /// A profile name, the key of a density budget. A lower-case letter, then lower-case
    /// letters, digits and hyphens.
    ProfileName, true, "", "profile name", |s| is_slug(s, '-'), SLUG
);
text_newtype!(
    /// A metric name: a lower-case letter, then lower-case letters, digits and underscores.
    MetricName, true, "", "metric name", |s| is_slug(s, '_'), r"^[a-z][a-z0-9_]*(?![\s\S])"
);
text_newtype!(
    /// A placeholder in a pattern: an upper-case letter, then upper-case letters, digits and
    /// underscores.
    PlaceholderName, true, "", "placeholder name", |s| {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }, r"^[A-Z][A-Z0-9_]*(?![\s\S])"
);
text_newtype!(
    /// An issue-qualified STE rule number such as `9:1.2` or `9:GR-2`.
    SteRef, true, "", "STE reference", is_ste_ref,
    r"^(?:[0-9]+:[0-9]+\.[0-9]+|[0-9]+:GR-[0-9]+)(?![\s\S])"
);

fn is_ste_ref(s: &str) -> bool {
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let Some((issue, rest)) = s.split_once(':') else {
        return false;
    };
    if !digits(issue) {
        return false;
    }
    match rest.strip_prefix("GR-") {
        Some(n) => digits(n),
        None => rest
            .split_once('.')
            .is_some_and(|(a, b)| digits(a) && digits(b)),
    }
}

/// A PCRE2 pattern (10.43 or later). It holds no `\uXXXX` or `\UXXXXXXXX` escape: write the
/// literal character or `\x{X...}`. The loader compiles the rest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Pcre2Pattern(String);

impl Pcre2Pattern {
    /// Validate and wrap a pattern.
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        if value.is_empty() || has_unicode_escape(&value) {
            Err(ValueError::new("PCRE2 pattern", value))
        } else {
            Ok(Self(value))
        }
    }

    /// The pattern source.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `pattern` holds a `\u` or `\U` escape that is not itself escaped.
fn has_unicode_escape(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.next(), Some('u' | 'U')) {
            return true;
        }
    }
    false
}

impl TryFrom<String> for Pcre2Pattern {
    type Error = ValueError;
    fn try_from(value: String) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl From<Pcre2Pattern> for String {
    fn from(p: Pcre2Pattern) -> Self {
        p.0
    }
}

impl JsonSchema for Pcre2Pattern {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Pcre2Pattern".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Matches when, reading escapes from the start, one escape is `\u` or `\U`. An escaped
        // backslash (`\\`) is one escape, so `\\u` is not a Unicode escape.
        schemars::json_schema!({
            "description": "A PCRE2 pattern (10.43 or later). The pattern may not contain a \\uXXXX \
                or \\UXXXXXXXX escape: write the literal character, or \\x{X...} in rule data \
                written for v3. A lookbehind must have a finite maximum length of at most 255 \
                characters, so + , * and {n,} are not allowed inside one. The schema rejects the \
                \\u and \\U escapes; the loader compiles the rest.",
            "type": "string",
            "minLength": 1,
            "not": {"pattern": r"^(?:[^\\]|\\[^uU])*\\[uU]"},
        })
    }
}

/// A category weight: a finite number, never negative.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Weight(f64);

impl Weight {
    /// The default weight, 1.
    pub const ONE: Self = Self(1.0);

    /// Wrap a weight, rejecting NaN, infinity and negatives.
    pub fn new(value: f64) -> Result<Self, ValueError> {
        if value.is_finite() && value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(ValueError::new("weight", value.to_string()))
        }
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Weight {
    type Error = ValueError;
    fn try_from(value: f64) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl From<Weight> for f64 {
    fn from(w: Weight) -> Self {
        w.0
    }
}

impl JsonSchema for Weight {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Weight".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "number", "minimum": 0, "default": 1.0})
    }
}

/// Which text units a lexical rule sees. prose is every prose block without code, URLs and
/// front matter. body-sentence is sentence minus heading and table blocks. raw is the unparsed
/// source and is for formatting rules only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(extend("default" = "prose"))]
pub enum Scope {
    /// Every prose block without code, URLs and front matter.
    #[default]
    Prose,
    /// Headings.
    Heading,
    /// Sentences.
    Sentence,
    /// Sentences outside headings and tables.
    BodySentence,
    /// Paragraphs.
    Paragraph,
    /// The whole document.
    Document,
    /// The unparsed source, for formatting rules.
    Raw,
}

/// A block or span kind the rule must not look inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BlockKind {
    /// A heading.
    Heading,
    /// A list item.
    ListItem,
    /// A code block.
    Code,
    /// Inline code.
    CodeSpan,
    /// A table.
    Table,
    /// A block quote.
    Quote,
    /// Front matter.
    FrontMatter,
}

/// Lint input surface, chosen by --mode. Its text context tag is code-comment (lint-report
/// Finding.contexts), a separate label that keeps its singular spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyMode {
    /// Prose files.
    Prose,
    /// Comments in source code.
    CodeComments,
}

/// A capitalization style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum CapitalizationStyle {
    /// Sentence case.
    Sentence,
    /// Title case.
    Title,
    /// All lower case.
    Lower,
    /// All upper case.
    Upper,
}

/// How a measured value compares with the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum Comparison {
    /// Greater than.
    #[default]
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
}

/// Where the `ai_signal` value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum AiSignalSource {
    /// Measured on a corpus.
    Measured,
    /// Taken from a catalog.
    Catalog,
    /// Not measured.
    #[default]
    Unmeasured,
}

/// The source of a placeholder's value. The only one is the locale allow list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(inline)]
pub enum PlaceholderSource {
    /// Words from the locale allow list.
    AllowedWords,
}

fn default_true() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(b: &bool) -> bool {
    *b
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Literal phrases, matched at word boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "TokensMatch")]
#[schemars(transform = x_extensions, transform = Kind("tokens"))]
pub struct TokensMatch {
    /// The phrases.
    pub tokens: UniqueVec<NonEmptyString, 1>,
    /// Match regardless of case.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    #[schemars(extend("default" = true))]
    pub ignore_case: bool,
    /// Report a match written entirely in capitals. Off by default because an all-caps token
    /// is usually a keyword, an identifier or an initialism. Applies to tokens, pattern and
    /// substitution.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("default" = false))]
    pub match_all_caps: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// A PCRE2 regular expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "PatternMatch")]
#[schemars(transform = x_extensions, transform = Kind("pattern"))]
pub struct PatternMatch {
    /// The pattern.
    pub pattern: Pcre2Pattern,
    /// Maps a literal placeholder string in the pattern to a configuration source. The only
    /// source is allowed-words: the loader replaces the placeholder with an alternation of the
    /// escaped ASCII alphabetic words from the locale allow list, sorted case-insensitively.
    /// With no words it becomes (?!), which never matches.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "non_empty_placeholders"
    )]
    #[schemars(transform = crate::schema::property_names, extend("minProperties" = 1))]
    pub placeholders: Option<BTreeMap<PlaceholderName, PlaceholderSource>>,
    /// Match regardless of case.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    #[schemars(extend("default" = true))]
    pub ignore_case: bool,
    /// Report a match written entirely in capitals.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("default" = false))]
    pub match_all_caps: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

#[allow(clippy::zero_sized_map_values)] // `PlaceholderSource` gains variants when a second source appears.
fn non_empty_placeholders<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BTreeMap<PlaceholderName, PlaceholderSource>>, D::Error> {
    let map = BTreeMap::deserialize(deserializer)?;
    if map.is_empty() {
        Err(de::Error::custom("placeholders must name at least one"))
    } else {
        Ok(Some(map))
    }
}

/// Pattern to replacement entries, in file order. File order breaks ties between keys of the
/// same length, so it is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitutions(Vec<(Pcre2Pattern, String)>);

impl Substitutions {
    /// The entries in file order.
    #[must_use]
    pub fn entries(&self) -> &[(Pcre2Pattern, String)] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Substitutions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Substitutions;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map from pattern to replacement")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Substitutions, A::Error> {
                let mut entries: Vec<(Pcre2Pattern, String)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<Pcre2Pattern, String>()? {
                    if entries.iter().any(|(e, _)| *e == k) {
                        return Err(de::Error::custom(format!(
                            "substitution key {:?} appears twice",
                            k.as_str()
                        )));
                    }
                    entries.push((k, v));
                }
                if entries.is_empty() {
                    return Err(de::Error::custom("substitutions must name at least one"));
                }
                Ok(Substitutions(entries))
            }
        }
        deserializer.deserialize_map(V)
    }
}

impl Serialize for Substitutions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

impl JsonSchema for Substitutions {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Substitutions".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "propertyNames": generator.subschema_for::<Pcre2Pattern>(),
            "additionalProperties": {"type": "string"},
            "minProperties": 1,
        })
    }
}

/// A map from a PCRE2 pattern to its replacement. The message names the replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "SubstitutionMatch")]
#[schemars(transform = x_extensions, transform = Kind("substitution"))]
pub struct SubstitutionMatch {
    /// Pattern to replacement.
    pub substitutions: Substitutions,
    /// Match regardless of case.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    #[schemars(extend("default" = true))]
    pub ignore_case: bool,
    /// Report a match written entirely in capitals.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("default" = false))]
    pub match_all_caps: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// A counted measurement compared with a threshold. The finding fires when measured
/// <comparison> threshold holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MetricMatch")]
#[schemars(transform = x_extensions, transform = Kind("metric"))]
pub struct MetricMatch {
    /// Metric name. The names the 2.x engine measures are listed in docs/contract/rules.md.
    pub metric: MetricName,
    /// The threshold.
    pub threshold: f64,
    /// How the measured value compares with the threshold.
    #[serde(default)]
    pub comparison: Comparison,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// A block-level shape check implemented by the engine and selected by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "StructureMatch")]
#[schemars(transform = x_extensions, transform = Kind("structure"))]
pub struct StructureMatch {
    /// The check name.
    pub check: Slug,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// Report text that is not written in the named capitalization style. New in 3.0: 2.x
/// expresses this as a pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "CapitalizationMatch")]
#[schemars(transform = x_extensions, transform = Kind("capitalization"))]
pub struct CapitalizationMatch {
    /// The style.
    pub style: CapitalizationStyle,
    /// Words exempt from the style, such as product names.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub allowed_words: Option<UniqueVec<NonEmptyString>>,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// A part-of-speech keyed controlled-vocabulary lookup selected by name (STE rule 1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "VocabularyMatch")]
#[schemars(transform = x_extensions, transform = Kind("vocabulary"))]
pub struct VocabularyMatch {
    /// The check name.
    pub check: Slug,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// A check no linter can run. Carried so writing and review guidance read one source. Never
/// produces a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "JudgementMatch")]
#[schemars(transform = x_extensions, transform = Kind("judgement"))]
pub struct JudgementMatch {
    /// The question the reviewer must answer. It must be decidable, not a matter of taste.
    pub question: NonEmptyString,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// The kind of a [`Match`], for filtering rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MatchKind {
    /// `tokens`.
    Tokens,
    /// `pattern`.
    Pattern,
    /// `substitution`.
    Substitution,
    /// `metric`.
    Metric,
    /// `structure`.
    Structure,
    /// `capitalization`.
    Capitalization,
    /// `vocabulary`.
    Vocabulary,
    /// `judgement`.
    Judgement,
}

/// The typed detection payload. match.kind selects the one allowed shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Match {
    /// Literal phrases.
    Tokens(TokensMatch),
    /// A PCRE2 pattern.
    Pattern(PatternMatch),
    /// A pattern to replacement map.
    Substitution(SubstitutionMatch),
    /// A counted measurement.
    Metric(MetricMatch),
    /// A block-level check.
    Structure(StructureMatch),
    /// A capitalization style.
    Capitalization(CapitalizationMatch),
    /// A controlled-vocabulary lookup.
    Vocabulary(VocabularyMatch),
    /// A question for a reviewer.
    Judgement(JudgementMatch),
}

/// Each payload is its own `$defs` entry, which names its `kind` tag.
impl JsonSchema for Match {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Match".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "The typed detection payload. match.kind selects the one allowed shape.",
            "oneOf": [
                generator.subschema_for::<TokensMatch>(),
                generator.subschema_for::<PatternMatch>(),
                generator.subschema_for::<SubstitutionMatch>(),
                generator.subschema_for::<MetricMatch>(),
                generator.subschema_for::<StructureMatch>(),
                generator.subschema_for::<CapitalizationMatch>(),
                generator.subschema_for::<VocabularyMatch>(),
                generator.subschema_for::<JudgementMatch>(),
            ],
        })
    }
}

impl Match {
    /// The kind of this payload.
    #[must_use]
    pub const fn kind(&self) -> MatchKind {
        match self {
            Self::Tokens(_) => MatchKind::Tokens,
            Self::Pattern(_) => MatchKind::Pattern,
            Self::Substitution(_) => MatchKind::Substitution,
            Self::Metric(_) => MatchKind::Metric,
            Self::Structure(_) => MatchKind::Structure,
            Self::Capitalization(_) => MatchKind::Capitalization,
            Self::Vocabulary(_) => MatchKind::Vocabulary,
            Self::Judgement(_) => MatchKind::Judgement,
        }
    }
}

/// Context tags that gate the rule. With only, the rule runs on text that carries at least one
/// listed tag. With skip, it is silent on text that carries any listed tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "ContextGateFields")]
#[schemars(rename = "Contexts")]
#[schemars(transform = x_extensions, extend("minProperties" = 1))]
pub struct ContextGate {
    /// Run only on text that carries one of these tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<UniqueVec<ContextTag>>,
    /// Stay silent on text that carries one of these tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<UniqueVec<ContextTag>>,
}

/// Context tags that gate the rule. With only, the rule runs on text that carries at least one
/// listed tag. With skip, it is silent on text that carries any listed tag.
#[derive(Deserialize, JsonSchema)]
#[schemars(rename = "Contexts")]
#[schemars(transform = x_extensions, extend("minProperties" = 1))]
struct ContextGateFields {
    #[serde(default, deserialize_with = "present")]
    only: Option<UniqueVec<ContextTag>>,
    #[serde(default, deserialize_with = "present")]
    skip: Option<UniqueVec<ContextTag>>,
    #[serde(flatten)]
    #[schemars(skip)]
    _ext: Extensions,
}

impl TryFrom<ContextGateFields> for ContextGate {
    type Error = String;
    fn try_from(f: ContextGateFields) -> Result<Self, String> {
        if f.only.is_none() && f.skip.is_none() {
            return Err("contexts must set `only` or `skip`".into());
        }
        Ok(Self {
            only: f.only,
            skip: f.skip,
        })
    }
}

/// Narrows where the rule is active. An absent key means no restriction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "AppliesToFields")]
#[schemars(rename = "AppliesTo")]
#[schemars(transform = x_extensions, extend("minProperties" = 1))]
pub struct AppliesTo {
    /// Lint modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<UniqueVec<ApplyMode, 1>>,
    /// Genres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genres: Option<UniqueVec<GenreId, 1>>,
}

/// Narrows where the rule is active. An absent key means no restriction.
#[derive(Deserialize, JsonSchema)]
#[schemars(rename = "AppliesTo")]
#[schemars(transform = x_extensions, extend("minProperties" = 1))]
struct AppliesToFields {
    #[serde(default, deserialize_with = "present")]
    modes: Option<UniqueVec<ApplyMode, 1>>,
    #[serde(default, deserialize_with = "present")]
    genres: Option<UniqueVec<GenreId, 1>>,
    #[serde(flatten)]
    #[schemars(skip)]
    _ext: Extensions,
}

impl TryFrom<AppliesToFields> for AppliesTo {
    type Error = String;
    fn try_from(f: AppliesToFields) -> Result<Self, String> {
        if f.modes.is_none() && f.genres.is_none() {
            return Err("applies_to must set `modes` or `genres`".into());
        }
        Ok(Self {
            modes: f.modes,
            genres: f.genres,
        })
    }
}

/// A named, closed reason a finding may be suppressed. A suppression annotation must cite one
/// by name. With detector, the engine drops the finding itself when the named detector, or the
/// context tag of that name, holds at the match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Exception")]
#[schemars(transform = x_extensions)]
pub struct Exception {
    /// The name a suppression annotation cites.
    pub name: Slug,
    /// A detector (or context tag) that drops the finding itself.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub detector: Option<Slug>,
    /// Prose description.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub description: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// Expected finding count, or an inclusive range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(rename = "Want", inline, transform = crate::schema::exclusive_alternatives)]
pub enum Want {
    /// Exactly this many findings.
    Count(u32),
    /// Between `min` and `max` findings, inclusive.
    Range(WantRange),
}

/// An inclusive finding-count range with at least one bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "WantRangeFields")]
#[schemars(rename = "WantRange")]
#[schemars(extend("additionalProperties" = false, "minProperties" = 1))]
pub struct WantRange {
    /// The lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    /// The upper bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "WantRange")]
#[schemars(extend("minProperties" = 1))]
struct WantRangeFields {
    #[serde(default, deserialize_with = "present")]
    min: Option<u32>,
    #[serde(default, deserialize_with = "present")]
    max: Option<u32>,
}

impl TryFrom<WantRangeFields> for WantRange {
    type Error = String;
    fn try_from(f: WantRangeFields) -> Result<Self, String> {
        if f.min.is_none() && f.max.is_none() {
            return Err("a range needs `min` or `max`".into());
        }
        Ok(Self {
            min: f.min,
            max: f.max,
        })
    }
}

/// A test case in the style of vale test. The input is linted with only this rule enabled.
/// want counts findings, contains and absent compare against the matched text and message of
/// those findings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "CaseFields")]
#[schemars(rename = "Case")]
#[schemars(transform = x_extensions,
extend(
        "anyOf" = [{"required": ["want"]}, {"required": ["contains"]}, {"required": ["absent"]}]
))]
pub struct Case {
    /// The text to lint with only this rule enabled.
    pub input: String,
    /// The expected finding count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub want: Option<Want>,
    /// Strings that each appear in the matched text or message of some finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<NonEmptyString>>,
    /// Strings that appear in no finding's matched text or message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent: Option<Vec<NonEmptyString>>,
    /// A note for the reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A test case in the style of vale test. The input is linted with only this rule enabled.
/// want counts findings, contains and absent compare against the matched text and message of
/// those findings.
#[derive(Deserialize, JsonSchema)]
#[schemars(rename = "Case")]
#[schemars(transform = x_extensions,
extend(
        "anyOf" = [{"required": ["want"]}, {"required": ["contains"]}, {"required": ["absent"]}]
))]
struct CaseFields {
    input: String,
    /// Expected finding count, or an inclusive range.
    #[serde(default, deserialize_with = "present")]
    want: Option<Want>,
    /// Each string appears in the matched text or message of some finding.
    #[serde(default, deserialize_with = "present")]
    #[schemars(extend("minItems" = 1))]
    contains: Option<Vec<NonEmptyString>>,
    /// No finding has a matched text or message that contains any of these strings.
    #[serde(default, deserialize_with = "present")]
    #[schemars(extend("minItems" = 1))]
    absent: Option<Vec<NonEmptyString>>,
    #[serde(default, deserialize_with = "present")]
    note: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    _ext: Extensions,
}

impl TryFrom<CaseFields> for Case {
    type Error = String;
    fn try_from(f: CaseFields) -> Result<Self, String> {
        let empty = |l: &Option<Vec<NonEmptyString>>| l.as_ref().is_some_and(Vec::is_empty);
        if empty(&f.contains) || empty(&f.absent) {
            return Err("`contains` and `absent` need at least one string".into());
        }
        if f.want.is_none() && f.contains.is_none() && f.absent.is_none() {
            return Err("a case needs `want`, `contains` or `absent`".into());
        }
        Ok(Self {
            input: f.input,
            want: f.want,
            contains: f.contains,
            absent: f.absent,
            note: f.note,
        })
    }
}

/// A before and after pair shown in rule documentation. For a mechanical rule the test runner
/// also asserts that bad produces a finding and good produces none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Example")]
#[schemars(transform = x_extensions)]
pub struct Example {
    /// Text the rule flags.
    pub bad: String,
    /// The fixed text, which the rule leaves alone.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub good: Option<String>,
    /// A note for the reader.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub note: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

/// Where the rule's idea comes from, so a reader can argue with it. Not the runtime
/// `RuleProvenance`, which names the pack that supplied a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Citation")]
#[schemars(transform = x_extensions)]
pub struct Citation {
    /// The source.
    pub source: NonEmptyString,
    /// Issue-qualified STE rule number such as 9:1.2 or 9:GR-2. A citation only.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub ste_ref: Option<SteRef>,
    /// Orwell reference.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub orwell_ref: Option<String>,
    /// A link.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub url: Option<String>,
    /// A note for the reader.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub note: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}

fn default_text_type() -> slopvac_text::TextType {
    slopvac_text::TextType::Any
}

/// The schema of `text_type`, shared as `$defs/TextType`.
struct TextTypeSchema;

impl JsonSchema for TextTypeSchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TextType".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Restrict the rule to sentences of one STE text type. any means the \
                rule does not care.",
            "enum": ["any", "procedural", "descriptive", "safety"],
            "default": "any",
        })
    }
}

/// A judgement rule is a suggestion and declares no exceptions or cases.
fn judgement_constraints() -> serde_json::Value {
    if_then(
        &serde_json::json!({
            "properties": {"match": {"properties": {"kind": {"const": "judgement"}}, "required": ["kind"]}},
            "required": ["match"]
        }),
        &serde_json::json!({
            "properties": {
                "severity": {"const": "suggestion"},
                "exceptions": {"maxItems": 0},
                "cases": {"maxItems": 0}
            }
        }),
    )
}

fn default_ai_signal() -> AiSignal {
    AiSignal::None
}

/// One check, as a category file embeds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RuleFields")]
#[schemars(rename = "Rule")]
#[schemars(transform = x_extensions, extend("allOf" = [judgement_constraints()]))]
pub struct Rule {
    /// Present only in a standalone rule file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<SchemaVersion>,
    /// The id inside its category.
    pub id: LocalRuleId,
    /// Short imperative label.
    pub name: NonEmptyString,
    /// Shown on a finding; `{match}` and `{replacement}` interpolate.
    pub message: NonEmptyString,
    /// The shipped level.
    pub severity: Severity,
    /// The text unit the rule sees.
    pub scope: Scope,
    /// The STE text type the rule is limited to.
    #[schemars(with = "TextTypeSchema")]
    pub text_type: slopvac_text::TextType,
    /// The detection payload.
    #[serde(rename = "match")]
    pub match_: Match,
    /// Block kinds the rule must not look inside.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_blocks: Option<UniqueVec<BlockKind>>,
    /// Context tags that gate the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contexts: Option<ContextGate>,
    /// Where the rule is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<AppliesTo>,
    /// Named reasons a finding may be suppressed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<UniqueVec<Exception>>,
    /// Literal strings that never fire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<UniqueVec<NonEmptyString>>,
    /// Before and after pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<Example>>,
    /// Test cases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cases: Option<Vec<Case>>,
    /// The rewrite operation, as guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// How strongly a hit indicates machine-written text.
    pub ai_signal: AiSignal,
    /// Where `ai_signal` comes from.
    pub ai_signal_source: AiSignalSource,
    /// Where the idea comes from.
    pub citation: Citation,
}

impl Rule {
    /// The examples, or none.
    #[must_use]
    pub fn examples(&self) -> &[Example] {
        self.examples.as_deref().unwrap_or_default()
    }

    /// The test cases, or none.
    #[must_use]
    pub fn cases(&self) -> &[Case] {
        self.cases.as_deref().unwrap_or_default()
    }

    /// The allowlist, or none.
    #[must_use]
    pub fn allowlist(&self) -> &[NonEmptyString] {
        self.allowlist.as_ref().map_or(&[], UniqueVec::as_slice)
    }

    /// The named exceptions, or none.
    #[must_use]
    pub fn exceptions(&self) -> &[Exception] {
        self.exceptions.as_ref().map_or(&[], UniqueVec::as_slice)
    }
}

/// A [`Rule`] as read: `severity` may be absent, and a judgement rule then gets `suggestion`.
#[derive(Deserialize, JsonSchema)]
#[schemars(rename = "Rule", description = "")]
#[schemars(transform = x_extensions, extend("allOf" = [judgement_constraints()]))]
struct RuleFields {
    #[serde(default, deserialize_with = "present")]
    schema_version: Option<SchemaVersion>,
    id: LocalRuleId,
    /// Short imperative label.
    name: NonEmptyString,
    /// Shown on a finding. Names the fix, not the violation. {match} and {replacement}
    /// interpolate.
    message: NonEmptyString,
    /// The shipped level. A category cap or a profile shift can lower it, never a rule.
    /// Disabling a rule is configuration, so there is no off level.
    #[serde(default, deserialize_with = "present")]
    #[schemars(extend("default" = "warning"))]
    severity: Option<Severity>,
    #[serde(default)]
    scope: Scope,
    #[serde(default = "default_text_type")]
    #[schemars(with = "TextTypeSchema")]
    text_type: slopvac_text::TextType,
    #[serde(rename = "match")]
    match_: Match,
    #[serde(default, deserialize_with = "present")]
    exclude_blocks: Option<UniqueVec<BlockKind>>,
    #[serde(default, deserialize_with = "present")]
    contexts: Option<ContextGate>,
    #[serde(default, deserialize_with = "present")]
    applies_to: Option<AppliesTo>,
    #[serde(default, deserialize_with = "present")]
    exceptions: Option<UniqueVec<Exception>>,
    /// Literal strings that never fire, even when the match holds.
    #[serde(default, deserialize_with = "present")]
    allowlist: Option<UniqueVec<NonEmptyString>>,
    #[serde(default, deserialize_with = "present")]
    examples: Option<Vec<Example>>,
    #[serde(default, deserialize_with = "present")]
    cases: Option<Vec<Case>>,
    /// The rewrite operation, as guidance.
    #[serde(default, deserialize_with = "present")]
    fix: Option<String>,
    #[serde(default = "default_ai_signal")]
    ai_signal: AiSignal,
    #[serde(default)]
    ai_signal_source: AiSignalSource,
    citation: Citation,
    #[serde(flatten)]
    #[schemars(skip)]
    _ext: Extensions,
}

impl TryFrom<RuleFields> for Rule {
    type Error = String;
    fn try_from(f: RuleFields) -> Result<Self, String> {
        let judgement = matches!(f.match_, Match::Judgement(_));
        let severity = match (judgement, f.severity) {
            (true, None | Some(Severity::Suggestion)) => Severity::Suggestion,
            (true, Some(_)) => {
                return Err("a judgement rule's severity must be `suggestion`".into());
            }
            (false, s) => s.unwrap_or(Severity::Warning),
        };
        if judgement && f.exceptions.as_ref().is_some_and(|e| !e.is_empty()) {
            return Err("a judgement rule declares no exceptions".into());
        }
        if judgement && f.cases.as_ref().is_some_and(|c| !c.is_empty()) {
            return Err("a judgement rule declares no cases".into());
        }
        Ok(Self {
            schema_version: f.schema_version,
            id: f.id,
            name: f.name,
            message: f.message,
            severity,
            scope: f.scope,
            text_type: f.text_type,
            match_: f.match_,
            exclude_blocks: f.exclude_blocks,
            contexts: f.contexts,
            applies_to: f.applies_to,
            exceptions: f.exceptions,
            allowlist: f.allowlist,
            examples: f.examples,
            cases: f.cases,
            fix: f.fix,
            ai_signal: f.ai_signal,
            ai_signal_source: f.ai_signal_source,
            citation: f.citation,
        })
    }
}

/// A standalone rule file: a [`Rule`] that also carries `schema_version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Rule", into = "Rule")]
pub struct RuleFile(Rule);

impl RuleFile {
    /// The rule.
    #[must_use]
    pub fn rule(&self) -> &Rule {
        &self.0
    }

    /// Take the rule, without its `schema_version`.
    #[must_use]
    pub fn into_rule(self) -> Rule {
        Rule {
            schema_version: None,
            ..self.0
        }
    }
}

impl TryFrom<Rule> for RuleFile {
    type Error = String;
    fn try_from(rule: Rule) -> Result<Self, String> {
        if rule.schema_version.is_some() {
            Ok(Self(rule))
        } else {
            Err("missing field `schema_version`".into())
        }
    }
}

impl From<RuleFile> for Rule {
    fn from(file: RuleFile) -> Self {
        file.0
    }
}

impl JsonSchema for RuleFile {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RuleFile".into()
    }

    /// The embedded rule shape, by reference, plus a required `schema_version`.
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = generator.subschema_for::<Rule>();
        schema.insert("required".into(), serde_json::json!(["schema_version"]));
        schema
    }
}

fn default_weight() -> Weight {
    Weight::ONE
}

/// A group of rules, and the unit users enable, disable and score by. One category per file.
/// Rule ids are unique within the category. Semantics are specified in docs/contract/rules.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Category")]
#[schemars(transform = x_extensions)]
pub struct Category {
    /// Always 3.
    pub schema_version: SchemaVersion,
    /// Category id. The qualified rule id is <pack>:<category>.<rule>.
    pub id: CategoryId,
    /// Display title.
    pub title: NonEmptyString,
    /// What the category covers.
    pub description: NonEmptyString,
    /// Contribution to the overall score before any config override.
    #[serde(default = "default_weight")]
    pub weight: Weight,
    /// Density budget per profile, keyed by profile name. Absent means no budget.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = crate::schema::property_names)]
    pub max_per_100_words: Option<BTreeMap<ProfileName, Density>>,
    /// Genres this category suits.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub recommended_for: Option<UniqueVec<GenreId>>,
    /// The rules.
    pub rules: Vec<Rule>,
    #[serde(flatten)]
    #[schemars(skip)]
    ext: Extensions,
}
