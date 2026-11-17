//! Structural taggers: built-in taggers that read more than one text unit.
//!
//! `tagger.schema.json` taggers read only the text of one unit. These taggers also read the
//! heading path, an admonition title, front matter, the file path or the block kind, and
//! they may tag a whole section or document. A pack file cannot say that, so the built-in
//! ones live in `builtin/structural.yaml`, an internal format and not a pack artifact.

use std::ops::Range;

use serde::Deserialize;

use crate::schema::is_name;
use crate::{BlockKind, BlockView, DocumentView, Error, Pattern, Tag, Tagger, normalize_path};

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
    /// The block has this kind. Code blocks are skipped unless named here.
    pub block_kind: Option<BlockKind>,
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

/// A structural tagger definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralSpec {
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

impl StructuralSpec {
    /// Parse one spec from YAML.
    pub fn from_yaml(text: &str) -> Result<Self, Error> {
        serde_saphyr::from_str(text).map_err(|e| Error::Parse(e.to_string()))
    }

    /// Compile into a tagger.
    pub fn compile(self) -> Result<StructuralTagger, Error> {
        if !is_name(&self.kind) {
            return Err(Error::Parse(format!(
                "tagger {}: kind `{}` is not a context tag (lowercase kebab-case)",
                self.id, self.kind
            )));
        }
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
            && w.block_kind.is_none()
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
        Ok(StructuralTagger {
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
///
/// Conditions combine with AND at three levels. Path and front matter are document
/// conditions. Heading path, admonition title and block kind are block conditions. The
/// sentence pattern is a sentence condition. Every given condition must hold for a tag.
pub struct StructuralTagger {
    spec: StructuralSpec,
    heading: Option<Pattern>,
    admonition: Option<Pattern>,
    path: Option<Pattern>,
    sentence: Option<Pattern>,
    fm: Option<(String, Option<Pattern>)>,
}

impl StructuralTagger {
    fn document_level_ok(&self, doc: &dyn DocumentView) -> Result<bool, Error> {
        if let Some(p) = &self.path
            && !p.is_match(&normalize_path(doc.path()))?
        {
            return Ok(false);
        }
        if let Some((key, pat)) = &self.fm {
            let Some(v) = doc.front_matter(key) else {
                return Ok(false);
            };
            if let Some(p) = pat
                && !p.is_match(v)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn block_ok(&self, b: &BlockView<'_>) -> Result<bool, Error> {
        let kind_ok = self
            .spec
            .when
            .block_kind
            .map_or_else(|| b.kind != BlockKind::CodeBlock, |k| b.kind == k);
        if !kind_ok {
            return Ok(false);
        }
        if let Some(p) = &self.heading {
            let mut any = false;
            for h in &b.heading_path {
                if p.is_match(h)? {
                    any = true;
                    break;
                }
            }
            if !any {
                return Ok(false);
            }
        }
        if let Some(p) = &self.admonition {
            match b.admonition_title {
                Some(t) if p.is_match(t)? => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn has_block_conditions(&self) -> bool {
        let w = &self.spec.when;
        w.heading_path.is_some()
            || w.admonition_title.is_some()
            || w.sentence.is_some()
            || w.block_kind.is_some()
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

impl Tagger for StructuralTagger {
    fn id(&self) -> &str {
        &self.spec.id
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        if !self.document_level_ok(doc)? {
            return Ok(Vec::new());
        }
        let whole = 0..doc.source_len();
        if self.spec.scope == Scope::Document && !self.has_block_conditions() {
            return Ok(vec![self.tag_for(whole, "document")]);
        }
        let blocks = doc.blocks();
        let mut out = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            if !self.block_ok(b)? {
                continue;
            }
            let hits: Vec<Range<usize>> = match &self.sentence {
                None => vec![b.span.clone()],
                Some(p) => {
                    let mut v = Vec::new();
                    for s in doc.sentences(b) {
                        if p.is_match(s.text)? {
                            v.push(s.span);
                        }
                    }
                    v
                }
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
                Scope::Document => return Ok(vec![self.tag_for(whole, "document")]),
            }
        }
        Ok(out)
    }
}

fn level(b: &BlockView<'_>) -> u8 {
    b.heading_level
        .unwrap_or_else(|| u8::try_from(b.heading_path.len()).unwrap_or(u8::MAX))
}

/// The section of block `i`: from its nearest heading up to the next heading of the same
/// or a shallower level. Text before the first heading forms a section of its own.
fn section_span(blocks: &[BlockView<'_>], i: usize) -> Range<usize> {
    let owner = (0..=i)
        .rev()
        .find(|&j| blocks[j].kind == BlockKind::Heading);
    let (first, own_level) = owner.map_or((0, 0), |j| (j, level(&blocks[j])));
    let last = (i + 1..blocks.len())
        .find(|&k| blocks[k].kind == BlockKind::Heading && level(&blocks[k]) <= own_level.max(1))
        .map_or(blocks.len() - 1, |k| k - 1);
    // Blocks that read out of source order (the cells of a wrapped table) can end after
    // the last block does, so the section runs to the furthest end among its blocks.
    let members = &blocks[first..=last];
    let start = members.iter().map(|b| b.span.start).min().unwrap_or(0);
    let end = members.iter().map(|b| b.span.end).max().unwrap_or(0);
    start..end
}
