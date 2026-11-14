//! Read-only view of a parsed document, as the taggers need it.
//!
//! STUB(slopvac-8l4.5.11): stands in for the `slopvac-model` document types.
//! Offsets are byte offsets into the original file bytes, as in
//! `docs/contract/positions.md`.

use std::ops::Range;

/// Block kinds a tagger can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// A heading.
    Heading,
    /// A paragraph.
    Paragraph,
    /// A list item.
    ListItem,
    /// A table row.
    TableRow,
    /// A fenced or indented code block.
    CodeBlock,
    /// A block quote.
    Quote,
}

/// One block of prose with its structural context.
#[derive(Debug, Clone)]
pub struct BlockView<'a> {
    /// Block kind.
    pub kind: BlockKind,
    /// Byte span of the block text in the file.
    pub span: Range<usize>,
    /// Block text.
    pub text: &'a str,
    /// Enclosing heading texts, outermost first. A heading block includes itself.
    pub heading_path: Vec<&'a str>,
    /// Title of the enclosing admonition, if any.
    pub admonition_title: Option<&'a str>,
}

/// A sentence inside a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentenceView<'a> {
    /// Byte span in the file.
    pub span: Range<usize>,
    /// Sentence text.
    pub text: &'a str,
}

/// What taggers read from a document.
pub trait DocumentView {
    /// File path as given on the command line.
    fn path(&self) -> &str;
    /// Document title (front matter `title` or the first heading).
    fn title(&self) -> Option<&str>;
    /// Value of a top-level front matter key, as text.
    fn front_matter(&self, key: &str) -> Option<&str>;
    /// Blocks in document order.
    fn blocks(&self) -> Vec<BlockView<'_>>;
    /// Sentences of one block.
    fn sentences<'a>(&self, block: &BlockView<'a>) -> Vec<SentenceView<'a>> {
        split_sentences(block)
    }
}

/// Split a block into sentences at `.`, `!` or `?` followed by whitespace.
#[must_use]
pub fn split_sentences<'a>(block: &BlockView<'a>) -> Vec<SentenceView<'a>> {
    let text = block.text;
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'.' | b'!' | b'?') {
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b')' | b'*' | b'_') {
                j += 1;
            }
            let at_end = j >= bytes.len();
            let spaced = !at_end && bytes[j].is_ascii_whitespace();
            let digit_dot = bytes[i] == b'.'
                && i > 0
                && bytes[i - 1].is_ascii_digit()
                && !at_end
                && bytes[j].is_ascii_digit();
            if (at_end || spaced) && !digit_dot {
                push(&mut out, block, start, j);
                start = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    push(&mut out, block, start, text.len());
    out
}

fn push<'a>(out: &mut Vec<SentenceView<'a>>, block: &BlockView<'a>, a: usize, b: usize) {
    let raw = &block.text[a..b];
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let lead = raw.len() - raw.trim_start().len();
    let s = block.span.start + a + lead;
    out.push(SentenceView {
        span: s..s + trimmed.len(),
        text: trimmed,
    });
}

/// Plain in-memory document for tests and tools.
#[derive(Debug, Default, Clone)]
pub struct PlainDocument {
    /// File path.
    pub path: String,
    /// Document title.
    pub title: Option<String>,
    /// Front matter pairs.
    pub front_matter: Vec<(String, String)>,
    /// Owned blocks.
    pub blocks: Vec<OwnedBlock>,
}

/// Owned counterpart of [`BlockView`].
#[derive(Debug, Clone)]
pub struct OwnedBlock {
    /// Block kind.
    pub kind: BlockKind,
    /// Byte span in the file.
    pub span: Range<usize>,
    /// Text.
    pub text: String,
    /// Heading path.
    pub heading_path: Vec<String>,
    /// Admonition title.
    pub admonition_title: Option<String>,
}

impl PlainDocument {
    /// Build a document from Markdown-ish text: ATX headings and blank-line
    /// separated paragraphs, `- ` list items. Enough for tests and `genre`.
    #[must_use]
    pub fn from_markdown(path: &str, src: &str) -> Self {
        let mut doc = PlainDocument {
            path: path.to_string(),
            ..Default::default()
        };
        let mut heads: Vec<(usize, String)> = Vec::new();
        let mut para: Option<(usize, usize)> = None;
        let mut offset = 0;
        let mut fence = false;
        let mut front_end = 0;
        if let Some(rest) = src.strip_prefix("---\n")
            && let Some(end) = rest.find("\n---")
        {
            for line in rest[..end].lines() {
                if let Some((k, v)) = line.split_once(':') {
                    doc.front_matter
                        .push((k.trim().to_string(), v.trim().trim_matches('"').to_string()));
                }
            }
            front_end = 4 + end + 4;
        }
        let flush = |doc: &mut PlainDocument,
                     para: &mut Option<(usize, usize)>,
                     heads: &[(usize, String)]| {
            if let Some((a, b)) = para.take() {
                doc.blocks.push(OwnedBlock {
                    kind: BlockKind::Paragraph,
                    span: a..b,
                    text: src[a..b].replace('\n', " "),
                    heading_path: heads.iter().map(|h| h.1.clone()).collect(),
                    admonition_title: None,
                });
            }
        };
        for line in src.split_inclusive('\n') {
            let start = offset;
            offset += line.len();
            if start < front_end {
                continue;
            }
            let content = line.trim_end_matches(['\n', '\r']);
            if content.trim_start().starts_with("```") {
                flush(&mut doc, &mut para, &heads);
                fence = !fence;
                continue;
            }
            if fence {
                continue;
            }
            if content.trim().is_empty() {
                flush(&mut doc, &mut para, &heads);
                continue;
            }
            let hashes = content.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) && content[hashes..].starts_with(' ') {
                flush(&mut doc, &mut para, &heads);
                let text = content[hashes..].trim().to_string();
                heads.retain(|h| h.0 < hashes);
                heads.push((hashes, text.clone()));
                if doc.title.is_none() {
                    doc.title = Some(text.clone());
                }
                let a = start + hashes + 1;
                doc.blocks.push(OwnedBlock {
                    kind: BlockKind::Heading,
                    span: a..a + text.len(),
                    text,
                    heading_path: heads.iter().map(|h| h.1.clone()).collect(),
                    admonition_title: None,
                });
                continue;
            }
            if let Some(rest) = content.strip_prefix("- ") {
                flush(&mut doc, &mut para, &heads);
                let a = start + 2;
                para = Some((a, a + rest.len()));
                continue;
            }
            let end = start + content.len();
            para = Some(match para {
                Some((a, _)) => (a, end),
                None => (start, end),
            });
        }
        flush(&mut doc, &mut para, &heads);
        doc
    }
}

impl DocumentView for PlainDocument {
    fn path(&self) -> &str {
        &self.path
    }
    fn title(&self) -> Option<&str> {
        self.front_matter("title").or(self.title.as_deref())
    }
    fn front_matter(&self, key: &str) -> Option<&str> {
        self.front_matter
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    fn blocks(&self) -> Vec<BlockView<'_>> {
        self.blocks
            .iter()
            .map(|b| BlockView {
                kind: b.kind,
                span: b.span.clone(),
                text: &b.text,
                heading_path: b.heading_path.iter().map(String::as_str).collect(),
                admonition_title: b.admonition_title.as_deref(),
            })
            .collect()
    }
}
