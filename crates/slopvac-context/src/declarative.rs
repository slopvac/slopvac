//! Declarative taggers from data packs.
//!
//! STUB(slopvac-8l4.5.12): field names follow the bead text of slopvac-8l4.2.4
//! until the pack schema is published.

use std::ops::Range;

use serde::Deserialize;

use crate::{BlockKind, BlockView, DocumentView, Error, Pattern, Tag, Tagger};

/// How far a matching condition spreads its tag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// The matching sentence.
    #[default]
    Sentence,
    /// The matching block.
    Paragraph,
    /// The enclosing heading section.
    Section,
    /// The whole document.
    Document,
}

/// Conditions of a declarative tagger. All given conditions must hold.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Any enclosing heading text matches.
    pub heading_path: Option<String>,
    /// The enclosing admonition title matches.
    pub admonition_title: Option<String>,
    /// Front matter key and a pattern for its value.
    pub frontmatter: Option<FrontmatterCondition>,
    /// The file path matches.
    pub path: Option<String>,
    /// The sentence matches.
    pub sentence: Option<String>,
}

/// Front matter condition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterCondition {
    /// Key name.
    pub key: String,
    /// Pattern for the value; absent means the key only has to exist.
    pub matches: Option<String>,
}

/// A tagger definition as written in a pack.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaggerSpec {
    /// Tagger id.
    pub id: String,
    /// Tag kind produced.
    pub kind: String,
    /// Tag scope.
    #[serde(default)]
    pub scope: Scope,
    /// Confidence in 0..=1.
    #[serde(default = "one")]
    pub confidence: f32,
    /// Conditions.
    pub when: When,
}

fn one() -> f32 {
    1.0
}

impl TaggerSpec {
    /// Parse one spec from YAML.
    pub fn from_yaml(text: &str) -> Result<Self, Error> {
        serde_saphyr::from_str(text).map_err(|e| Error::Parse(e.to_string()))
    }

    /// Compile into a tagger.
    pub fn compile(self) -> Result<DeclarativeTagger, Error> {
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(Error::Parse(format!(
                "tagger {}: confidence outside 0..=1",
                self.id
            )));
        }
        let w = &self.when;
        if w.heading_path.is_none()
            && w.admonition_title.is_none()
            && w.frontmatter.is_none()
            && w.path.is_none()
            && w.sentence.is_none()
        {
            return Err(Error::Parse(format!(
                "tagger {}: `when` has no condition",
                self.id
            )));
        }
        let c = |o: &Option<String>| o.as_deref().map(Pattern::new_caseless).transpose();
        let fm = match &w.frontmatter {
            Some(f) => Some((
                f.key.clone(),
                f.matches
                    .as_deref()
                    .map(Pattern::new_caseless)
                    .transpose()?,
            )),
            None => None,
        };
        Ok(DeclarativeTagger {
            heading: c(&w.heading_path)?,
            admonition: c(&w.admonition_title)?,
            path: c(&w.path)?,
            sentence: c(&w.sentence)?,
            fm,
            spec: self,
        })
    }
}

/// A compiled declarative tagger.
pub struct DeclarativeTagger {
    spec: TaggerSpec,
    heading: Option<Pattern>,
    admonition: Option<Pattern>,
    path: Option<Pattern>,
    sentence: Option<Pattern>,
    fm: Option<(String, Option<Pattern>)>,
}

impl DeclarativeTagger {
    fn document_level_ok(&self, doc: &dyn DocumentView) -> bool {
        if let Some(p) = &self.path
            && !p.is_match(doc.path())
        {
            return false;
        }
        if let Some((key, pat)) = &self.fm {
            match (doc.front_matter(key), pat) {
                (None, _) => return false,
                (Some(v), Some(p)) if !p.is_match(v) => return false,
                _ => {}
            }
        }
        true
    }

    fn block_ok(&self, b: &BlockView<'_>) -> bool {
        if let Some(p) = &self.heading
            && !b.heading_path.iter().any(|h| p.is_match(h))
        {
            return false;
        }
        if let Some(p) = &self.admonition
            && !b.admonition_title.is_some_and(|t| p.is_match(t))
        {
            return false;
        }
        true
    }

    fn tag_for(&self, span: Range<usize>, evidence: &str) -> Tag {
        Tag {
            kind: self.spec.kind.clone(),
            span,
            confidence: self.spec.confidence,
            evidence: format!("{}: {evidence}", self.spec.id),
        }
    }
}

impl Tagger for DeclarativeTagger {
    fn id(&self) -> &str {
        &self.spec.id
    }

    fn tag(&self, doc: &dyn DocumentView) -> Vec<Tag> {
        if !self.document_level_ok(doc) {
            return Vec::new();
        }
        let blocks = doc.blocks();
        let doc_end = blocks.iter().map(|b| b.span.end).max().unwrap_or(0);
        let mut out = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            if b.kind == BlockKind::CodeBlock || !self.block_ok(b) {
                continue;
            }
            let hits: Vec<Range<usize>> = match &self.sentence {
                None => vec![b.span.clone()],
                Some(p) => doc
                    .sentences(b)
                    .into_iter()
                    .filter(|s| p.is_match(s.text))
                    .map(|s| s.span)
                    .collect(),
            };
            if hits.is_empty() {
                continue;
            }
            match self.spec.scope {
                Scope::Sentence => {
                    out.extend(hits.into_iter().map(|s| self.tag_for(s, "sentence")));
                }
                Scope::Paragraph => out.push(self.tag_for(b.span.clone(), "block")),
                Scope::Section => out.push(self.tag_for(section_span(&blocks, i), "section")),
                Scope::Document => return vec![self.tag_for(0..doc_end, "document")],
            }
        }
        out
    }
}

fn section_span(blocks: &[BlockView<'_>], i: usize) -> Range<usize> {
    let path = &blocks[i].heading_path;
    let first = (0..=i)
        .rev()
        .find(|&j| blocks[j].kind == BlockKind::Heading && blocks[j].heading_path == *path)
        .unwrap_or(i);
    let last = (i..blocks.len())
        .take_while(|&j| blocks[j].heading_path.starts_with(path))
        .last()
        .unwrap_or(i);
    blocks[first].span.start..blocks[last].span.end
}
