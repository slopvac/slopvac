//! Structural conditions of a declarative tagger: the `when` object of
//! `tagger.schema.json`.
//!
//! A tagger with `match` reads the text of one unit. A tagger with `when` also reads the
//! heading path, an admonition title, front matter, the file path or the block kind, and
//! it may tag a whole section or document.

use std::ops::Range;

use serde::Deserialize;

use crate::schema::{Extensions, present};
use crate::{BlockKind, BlockView, DocumentView, Error, Pattern, Tag, normalize_path};

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

/// Conditions of a structural tagger. All given conditions must hold.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct When {
    /// Any enclosing heading text matches.
    #[serde(default, deserialize_with = "present")]
    pub heading_path: Option<String>,
    /// The enclosing admonition title matches.
    #[serde(default, deserialize_with = "present")]
    pub admonition_title: Option<String>,
    /// Front matter key and a pattern for its value.
    #[serde(default, deserialize_with = "present")]
    pub front_matter: Option<FrontMatterCondition>,
    /// The file path matches.
    #[serde(default, deserialize_with = "present")]
    pub path: Option<String>,
    /// The sentence matches.
    #[serde(default, deserialize_with = "present")]
    pub sentence: Option<String>,
    /// The block has this kind. Code blocks are skipped unless named here.
    #[serde(default, deserialize_with = "present")]
    pub block_kind: Option<BlockKind>,
    #[serde(flatten)]
    _extensions: Extensions,
}

/// Front matter condition.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontMatterCondition {
    /// Key name.
    pub key: String,
    /// Pattern for the value; absent means the key only has to exist.
    #[serde(default, deserialize_with = "present")]
    pub matches: Option<String>,
    #[serde(flatten)]
    _extensions: Extensions,
}

/// The parts of a tagger file that a structural tagger needs.
pub(crate) struct Settings<'a> {
    pub id: &'a str,
    pub tag: &'a str,
    pub confidence: f32,
    pub evidence: Option<&'a str>,
    pub scope: Scope,
}

impl When {
    fn is_empty(&self) -> bool {
        self.heading_path.is_none()
            && self.admonition_title.is_none()
            && self.front_matter.is_none()
            && self.path.is_none()
            && self.sentence.is_none()
            && self.block_kind.is_none()
    }

    /// Compile into a tagger.
    pub(crate) fn compile(&self, s: &Settings<'_>) -> Result<StructuralTagger, Error> {
        let fail = |msg: &str| Error::Parse(format!("tagger `{}`: {msg}", s.id));
        if self.is_empty() {
            return Err(fail("`when` needs at least one condition"));
        }
        let strings = [
            &self.heading_path,
            &self.admonition_title,
            &self.path,
            &self.sentence,
        ];
        let fm = self.front_matter.as_ref();
        if strings.iter().any(|o| o.as_deref() == Some(""))
            || fm.is_some_and(|f| f.key.is_empty() || f.matches.as_deref() == Some(""))
        {
            return Err(fail("`when` conditions need at least one character"));
        }
        let c = |o: &Option<String>| o.as_deref().map(Pattern::new_caseless).transpose();
        let fm = match fm {
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
            id: s.id.to_owned(),
            tag: s.tag.to_owned(),
            confidence: s.confidence,
            evidence: s.evidence.map(str::to_owned),
            scope: s.scope,
            block_kind: self.block_kind,
            has_block_conditions: self.heading_path.is_some()
                || self.admonition_title.is_some()
                || self.sentence.is_some()
                || self.block_kind.is_some(),
            heading: c(&self.heading_path)?,
            admonition: c(&self.admonition_title)?,
            path: c(&self.path)?,
            sentence: c(&self.sentence)?,
            fm,
        })
    }
}

/// A compiled structural tagger.
///
/// Conditions combine with AND at three levels. Path and front matter are document
/// conditions. Heading path, admonition title and block kind are block conditions. The
/// sentence pattern is a sentence condition. Every given condition must hold for a tag.
pub(crate) struct StructuralTagger {
    id: String,
    tag: String,
    confidence: f32,
    evidence: Option<String>,
    scope: Scope,
    block_kind: Option<BlockKind>,
    has_block_conditions: bool,
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
        let kind_ok = match self.block_kind {
            Some(k) => b.kind == k,
            None => b.kind != BlockKind::CodeBlock,
        };
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

    fn tag_for(&self, span: Range<usize>, evidence: &str) -> Tag {
        Tag {
            kind: self.tag.clone(),
            span,
            confidence: self.confidence,
            evidence: self
                .evidence
                .clone()
                .unwrap_or_else(|| format!("{}: {evidence}", self.id)),
        }
    }
}

impl StructuralTagger {
    pub(crate) fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        if !self.document_level_ok(doc)? {
            return Ok(Vec::new());
        }
        let whole = 0..doc.source_len();
        if self.scope == Scope::Document && !self.has_block_conditions {
            return Ok(vec![self.tag_for(whole, "document")]);
        }
        let blocks = doc.blocks();
        let mut out = Vec::new();
        let mut bounds = None;
        let mut emitted: Vec<bool> = Vec::new();
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
            match self.scope {
                Scope::Sentence => {
                    out.extend(hits.into_iter().map(|s| self.tag_for(s, "sentence")));
                }
                Scope::Paragraph => out.push(self.tag_for(b.span.clone(), "block")),
                Scope::Section => {
                    let (spans, owner_of) = bounds.get_or_insert_with(|| {
                        let (spans, owner_of) = sections(&blocks);
                        emitted = vec![false; spans.len()];
                        (spans, owner_of)
                    });
                    if let Some(&id) = owner_of.get(i)
                        && let Some(done) = emitted.get_mut(id)
                        && !std::mem::replace(done, true)
                        && let Some(span) = spans.get(id)
                    {
                        out.push(self.tag_for(span.clone(), "section"));
                    }
                }
                Scope::Document => return Ok(vec![self.tag_for(whole, "document")]),
            }
        }
        Ok(out)
    }
}

/// Count of blocks the section bounds routine has examined on this thread.
#[doc(hidden)]
pub mod work {
    use std::cell::Cell;

    thread_local! {
        static STEPS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn step(n: usize) {
        STEPS.with(|c| c.set(c.get() + n));
    }

    /// Blocks examined by section bounds since the last call, for tests that bound the work.
    #[doc(hidden)]
    #[must_use]
    pub fn take_section_steps() -> usize {
        STEPS.with(|c| c.replace(0))
    }
}

fn level(b: &BlockView<'_>) -> u8 {
    b.heading_level
        .unwrap_or_else(|| u8::try_from(b.heading_path.len()).unwrap_or(u8::MAX))
}

/// The section of every block: from its nearest heading up to the next heading of the same
/// or a shallower level. Text before the first heading forms a section of its own.
///
/// Returns the distinct section spans and, for each block, the index of its section. A
/// section's span runs to the furthest end among its blocks, because blocks that read out of
/// source order (the cells of a wrapped table) can end after the last block does. One pass
/// with a stack of open sections: a block lands in the innermost one, and a closing section
/// folds its extent into the one that encloses it.
fn sections(blocks: &[BlockView<'_>]) -> (Vec<Range<usize>>, Vec<usize>) {
    struct Open {
        /// Effective level: the next heading at this level or shallower closes it.
        level: u8,
        id: usize,
        extent: Option<Range<usize>>,
    }
    fn widen(extent: &mut Option<Range<usize>>, with: &Range<usize>) {
        *extent = Some(match extent.take() {
            Some(e) => e.start.min(with.start)..e.end.max(with.end),
            None => with.clone(),
        });
    }
    // Close the innermost section: record its span and fold it into the enclosing one.
    fn close(open: &mut Vec<Open>, spans: &mut [Range<usize>]) {
        let Some(done) = open.pop() else { return };
        let Some(extent) = done.extent else { return };
        if let Some(slot) = spans.get_mut(done.id) {
            slot.clone_from(&extent);
        }
        if let Some(parent) = open.last_mut() {
            widen(&mut parent.extent, &extent);
        }
    }
    // Section 0 is the text before the first heading.
    let mut spans: Vec<Range<usize>> = Vec::new();
    spans.push(0..0);
    let mut owner_of = Vec::with_capacity(blocks.len());
    // The text before the first heading closes at the first heading of level 1 or less.
    let mut open = vec![Open {
        level: 1,
        id: 0,
        extent: None,
    }];
    for b in blocks {
        work::step(1);
        if b.kind == BlockKind::Heading {
            let l = level(b);
            while open.last().is_some_and(|top| top.level >= l) {
                close(&mut open, &mut spans);
            }
            open.push(Open {
                level: l.max(1),
                id: spans.len(),
                extent: None,
            });
            spans.push(0..0);
        }
        // `open` is never empty here: a heading was just pushed, and a non-heading finds
        // the section it was pushed to, or the prelude, which only a heading closes.
        if let Some(top) = open.last_mut() {
            widen(&mut top.extent, &b.span);
            owner_of.push(top.id);
        }
    }
    // Sections still open at the end of the document close there.
    while !open.is_empty() {
        close(&mut open, &mut spans);
    }
    (spans, owner_of)
}
