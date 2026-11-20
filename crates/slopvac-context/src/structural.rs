//! Structural conditions of a declarative tagger: the `when` object of
//! `tagger.schema.json`.
//!
//! A tagger with `match` reads the text of one unit. A tagger with `when` also reads the
//! heading path, an admonition title, front matter, the file path or the block kind, and
//! it may tag a whole section or document.

use std::collections::HashMap;
use std::ops::Range;

use serde::Deserialize;

use crate::schema::{Extensions, present};
use crate::{BlockKind, BlockView, DocumentView, Error, Pattern, Tag, TagSink, normalize_path};

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

    /// Follow the heading path to block `b`.
    ///
    /// `verdicts` holds one entry per heading in the current path, outermost first. Blocks
    /// come in document order and a block's heading path changes only at a heading block, so
    /// the entries below the point where the path changed stay valid. A new entry starts
    /// unknown: the heading pattern runs on a heading only when a block that could be tagged
    /// needs the verdict, and then the verdict serves every block below it. Call this for
    /// every block, in order, even when the block is later skipped.
    fn enter(&self, b: &BlockView<'_>, verdicts: &mut Vec<Option<bool>>) {
        if self.heading.is_none() {
            return;
        }
        // A heading block ends its own path, so its last entry is the one to compute anew.
        let keep = if b.kind == BlockKind::Heading {
            b.heading_path.len().saturating_sub(1)
        } else {
            b.heading_path.len()
        };
        verdicts.truncate(keep);
        verdicts.resize(b.heading_path.len(), None);
    }

    /// Whether any heading above `b` matches the heading pattern. Headings are tried
    /// outermost first and the walk stops at the first match, so an error from a heading
    /// below a matching one never surfaces.
    fn heading_ok(&self, b: &BlockView<'_>, verdicts: &mut [Option<bool>]) -> Result<bool, Error> {
        let Some(p) = &self.heading else {
            return Ok(true);
        };
        for (h, slot) in b.heading_path.iter().zip(verdicts.iter_mut()) {
            let hit = if let Some(hit) = *slot {
                hit
            } else {
                crate::work::heading(h.len());
                let hit = p.is_match(h)?;
                *slot = Some(hit);
                hit
            };
            if hit {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether the admonition pattern matches `title`, remembered per title.
    ///
    /// A view that reports many blocks under one admonition lends them all the same title, so
    /// the title's address and length identify it, and the pattern runs once per title instead
    /// of once per block. The key is only compared, never read through. A title that is equal
    /// text at another address gets its own entry, which costs a repeat match and nothing more.
    fn admonition_ok(
        p: &Pattern,
        title: &str,
        seen: &mut HashMap<(usize, usize), bool>,
    ) -> Result<bool, Error> {
        let key = (title.as_ptr().addr(), title.len());
        if let Some(&hit) = seen.get(&key) {
            return Ok(hit);
        }
        crate::work::admonition(title.len());
        let hit = p.is_match(title)?;
        seen.insert(key, hit);
        Ok(hit)
    }

    fn block_ok(
        &self,
        b: &BlockView<'_>,
        verdicts: &mut [Option<bool>],
        titles: &mut HashMap<(usize, usize), bool>,
    ) -> Result<bool, Error> {
        let kind_ok = match self.block_kind {
            Some(k) => b.kind == k,
            None => b.kind != BlockKind::CodeBlock,
        };
        if !kind_ok || !self.heading_ok(b, verdicts)? {
            return Ok(false);
        }
        if let Some(p) = &self.admonition {
            match b.admonition_title {
                Some(t) if Self::admonition_ok(p, t, titles)? => {}
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
    pub(crate) fn tag(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        if !self.document_level_ok(doc)? {
            return Ok(());
        }
        let whole = 0..doc.source_len();
        if self.scope == Scope::Document && !self.has_block_conditions {
            return sink.push_with(|| self.tag_for(whole, "document"));
        }
        let blocks = doc.blocks();
        let mut bounds = None;
        let mut emitted: Vec<bool> = Vec::new();
        let mut verdicts = Vec::new();
        let mut titles = HashMap::new();
        for (i, b) in blocks.iter().enumerate() {
            self.enter(b, &mut verdicts);
            if !self.block_ok(b, &mut verdicts, &mut titles)? {
                continue;
            }
            let hits: Vec<Range<usize>> = match &self.sentence {
                None if self.scope == Scope::Sentence => {
                    doc.sentences(b).into_iter().map(|s| s.span).collect()
                }
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
                    for s in hits {
                        sink.push_with(|| self.tag_for(s, "sentence"))?;
                    }
                }
                Scope::Paragraph => sink.push_with(|| self.tag_for(b.span.clone(), "block"))?,
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
                        sink.push_with(|| self.tag_for(span.clone(), "section"))?;
                    }
                }
                Scope::Document => return sink.push_with(|| self.tag_for(whole, "document")),
            }
        }
        Ok(())
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
        crate::work::section(1);
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
