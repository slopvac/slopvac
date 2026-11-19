//! Declarative taggers from data packs: the `tagger` artifact of
//! `schemas/v3/tagger.schema.json`, specified in `docs/contract/plugins.md`, section 5.
//!
//! A tagger with `match` sets one context tag on each text unit where a `match` entry
//! matches and no `unless` entry matches. It reads the text of the unit and nothing else, so
//! its tag span is the whole unit. A tagger with `when` reads document structure instead
//! (see `structural.rs`).

use std::ops::Range;

use serde::Deserialize;

use crate::schema::{Extensions, check_schema_version, is_name, present};
use crate::structural::{Scope, Settings, StructuralTagger, When};
use crate::{BlockKind, BlockView, DocumentView, Error, Pattern, Tag, TagSink, Tagger};

/// The text unit that receives the tag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
    /// One sentence of any block except a code block.
    #[default]
    Sentence,
    /// A paragraph block.
    Paragraph,
    /// A heading block.
    Heading,
    /// A list item block.
    ListItem,
    /// A table cell block.
    TableCell,
}

impl Unit {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sentence => "sentence",
            Self::Paragraph => "paragraph",
            Self::Heading => "heading",
            Self::ListItem => "list-item",
            Self::TableCell => "table-cell",
        }
    }

    /// The block kind that holds this unit, or `None` for sentences, which any prose
    /// block holds.
    fn block_kind(self) -> Option<BlockKind> {
        match self {
            Self::Sentence => None,
            Self::Paragraph => Some(BlockKind::Paragraph),
            Self::Heading => Some(BlockKind::Heading),
            Self::ListItem => Some(BlockKind::ListItem),
            Self::TableCell => Some(BlockKind::TableRow),
        }
    }
}

/// One condition on a text unit: a PCRE2 pattern, or a list of words and phrases.
#[derive(Debug, Clone, Deserialize)]
pub struct Matcher {
    /// PCRE2 pattern. Exactly one of `pattern` and `words` is set.
    #[serde(default, deserialize_with = "present")]
    pub pattern: Option<String>,
    /// Words and phrases, matched as whole words.
    #[serde(default, deserialize_with = "present")]
    pub words: Option<Vec<String>>,
    /// Match case. Matching ignores case when false.
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(flatten)]
    _extensions: Extensions,
}

/// A declarative tagger as written in a pack file.
#[derive(Debug, Clone, Deserialize)]
pub struct TaggerSpec {
    /// Must be 3.
    pub schema_version: u32,
    /// Tagger id, `^[a-z][a-z0-9-]*$`.
    pub id: String,
    /// What the tagger does, for people.
    #[serde(default, deserialize_with = "present")]
    pub description: Option<String>,
    /// The context tag the tagger sets.
    pub tag: String,
    /// The unit that receives the tag. Not with `when`.
    #[serde(default, deserialize_with = "present")]
    pub unit: Option<Unit>,
    /// The tagger fires when at least one entry matches the unit. Not with `when`.
    #[serde(default, rename = "match", deserialize_with = "present")]
    pub matches: Option<Vec<Matcher>>,
    /// The tagger stays silent for a unit where any of these entries matches. Not with `when`.
    #[serde(default, deserialize_with = "present")]
    pub unless: Option<Vec<Matcher>>,
    /// Structural conditions, instead of `match`.
    #[serde(default, deserialize_with = "present")]
    pub when: Option<When>,
    /// With `when`: how far the tag spreads. Defaults to a sentence.
    #[serde(default, deserialize_with = "present")]
    pub scope: Option<Scope>,
    /// Confidence in 0..=1.
    #[serde(default = "one")]
    pub confidence: f64,
    /// Short text that explains the tag in reports.
    #[serde(default, deserialize_with = "present")]
    pub evidence: Option<String>,
    #[serde(flatten)]
    _extensions: Extensions,
}

fn one() -> f64 {
    1.0
}

/// A confidence that `compile` checked to lie in 0..=1. The tag type stores single
/// precision, and rounding a value in that range stays in range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is checked to lie in 0..=1, where f32 rounds and never overflows"
)]
fn to_f32(confidence: f64) -> f32 {
    confidence as f32
}

impl TaggerSpec {
    /// Parse one tagger from YAML.
    pub fn from_yaml(text: &str) -> Result<Self, Error> {
        serde_saphyr::from_str(text).map_err(Error::Yaml)
    }

    /// Parse a YAML list of taggers.
    pub fn list_from_yaml(text: &str) -> Result<Vec<Self>, Error> {
        serde_saphyr::from_str(text).map_err(Error::Yaml)
    }

    /// Check the rules that the schema states and compile the patterns.
    pub fn compile(self) -> Result<DeclarativeTagger, Error> {
        let id = self.id.clone();
        let fail = |msg: &str| Error::Parse(format!("tagger `{id}`: {msg}"));
        check_schema_version(&format!("tagger `{id}`"), self.schema_version)?;
        if !is_name(&self.id) {
            return Err(fail("id must be lowercase kebab-case"));
        }
        if !is_name(&self.tag) {
            return Err(fail("tag must be a context tag (lowercase kebab-case)"));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(fail("confidence is outside 0..=1"));
        }
        if self.evidence.as_deref().is_some_and(str::is_empty)
            || self.description.as_deref().is_some_and(str::is_empty)
        {
            return Err(fail("evidence and description need at least one character"));
        }
        if let Some(when) = &self.when {
            if self.matches.is_some() || self.unless.is_some() || self.unit.is_some() {
                return Err(fail("`when` excludes `match`, `unless` and `unit`"));
            }
            let structural = when.compile(&Settings {
                id: &id,
                tag: &self.tag,
                confidence: to_f32(self.confidence),
                evidence: self.evidence.as_deref(),
                scope: self.scope.unwrap_or_default(),
            })?;
            return Ok(DeclarativeTagger {
                spec: self,
                matching: Vec::new(),
                unless: Vec::new(),
                structural: Some(structural),
            });
        }
        if self.scope.is_some() {
            return Err(fail("`scope` needs `when`"));
        }
        let entries = self.matches.as_deref().unwrap_or_default();
        if entries.is_empty() {
            return Err(fail("`match` needs at least one entry"));
        }
        let matching = entries
            .iter()
            .map(|m| m.compile(&id))
            .collect::<Result<Vec<_>, _>>()?;
        let unless = self
            .unless
            .iter()
            .flatten()
            .map(|m| m.compile(&id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DeclarativeTagger {
            spec: self,
            matching,
            unless,
            structural: None,
        })
    }
}

impl Matcher {
    fn compile(&self, tagger: &str) -> Result<Pattern, Error> {
        let fail = |msg: &str| Error::Parse(format!("tagger `{tagger}`: {msg}"));
        let source = match (&self.pattern, &self.words) {
            (Some(p), None) if p.is_empty() => return Err(fail("pattern is empty")),
            (Some(p), None) => p.clone(),
            (None, Some(w)) => words_pattern(w)
                .ok_or_else(|| fail("`words` needs at least one distinct, non-empty entry"))?,
            _ => return Err(fail("a matcher takes exactly one of `pattern` and `words`")),
        };
        if self.case_sensitive {
            Pattern::new(&source)
        } else {
            Pattern::new_caseless(&source)
        }
    }
}

/// A PCRE2 pattern for whole words and phrases, with Unicode-aware word boundaries. `None` for an empty list, an empty entry
/// or a repeated entry.
fn words_pattern(words: &[String]) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    if words.is_empty() || words.iter().any(|w| w.is_empty() || !seen.insert(w)) {
        return None;
    }
    let alternatives: Vec<String> = words.iter().map(|w| escape(w)).collect();
    Some(format!(
        r"(*UCP)(?<![\w\p{{M}}])(?:{})(?![\w\p{{M}}])",
        alternatives.join("|")
    ))
}

fn escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if !c.is_alphanumeric() && c != '_' && c.is_ascii() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A compiled declarative tagger.
pub struct DeclarativeTagger {
    spec: TaggerSpec,
    matching: Vec<Pattern>,
    unless: Vec<Pattern>,
    structural: Option<StructuralTagger>,
}

impl DeclarativeTagger {
    fn fires(&self, text: &str) -> Result<bool, Error> {
        let mut any = false;
        for p in &self.matching {
            if p.is_match(text)? {
                any = true;
                break;
            }
        }
        if !any {
            return Ok(false);
        }
        for p in &self.unless {
            if p.is_match(text)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn unit(&self) -> Unit {
        self.spec.unit.unwrap_or_default()
    }

    fn tag_for(&self, span: Range<usize>) -> Tag {
        Tag {
            kind: self.spec.tag.clone(),
            span,
            confidence: to_f32(self.spec.confidence),
            evidence: self
                .spec
                .evidence
                .clone()
                .unwrap_or_else(|| format!("{}: {}", self.spec.id, self.unit().as_str())),
        }
    }

    fn tag_block(
        &self,
        doc: &dyn DocumentView,
        b: &BlockView<'_>,
        sink: &mut TagSink,
    ) -> Result<(), Error> {
        match self.unit().block_kind() {
            Some(kind) if b.kind == kind => {
                if self.fires(b.text)? {
                    sink.push_with(|| self.tag_for(b.span.clone()))?;
                }
            }
            Some(_) => {}
            None if b.kind == BlockKind::CodeBlock => {}
            None => {
                for s in doc.sentences(b) {
                    if self.fires(s.text)? {
                        sink.push_with(|| self.tag_for(s.span))?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Tagger for DeclarativeTagger {
    fn id(&self) -> &str {
        &self.spec.id
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        let mut sink = TagSink::unbounded();
        self.tag_into(doc, &mut sink)?;
        Ok(sink.into_tags())
    }

    fn tag_into(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        if let Some(structural) = &self.structural {
            return structural.tag(doc, sink);
        }
        for b in doc.blocks() {
            self.tag_block(doc, &b, sink)?;
        }
        Ok(())
    }
}
