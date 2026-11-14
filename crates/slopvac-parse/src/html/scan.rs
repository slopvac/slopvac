//! The tokenizer layer: html5gum drives html5ever's tree builder.
//!
//! html5gum is a WHATWG-compliant tokenizer that reports source positions. html5ever's
//! tree builder is the spec's tree construction, and it takes the tokens html5ever's own
//! tokenizer would produce. This emitter turns html5gum's events into those tokens, feeds
//! them to the tree builder one by one, and passes the builder's request for a tokenizer
//! state (raw text, RCDATA, script data, plaintext) back to html5gum.
//!
//! Each source character of text goes to the tree builder as a stand-in, and the queue in
//! [`Dom`] remembers its source range. See the `dom` module.

use std::rc::Rc;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    Doctype, Tag, TagKind, Token, TokenSink, TokenSinkResult, TokenSinkResult as Sinked,
};
use html5ever::tree_builder::{Tracer, TreeBuilder, TreeBuilderOpts};
use html5ever::{Attribute, LocalName, QualName, ns};
use html5gum::{Emitter, Error, State, Tokenizer, naive_next_state};

use super::dom::{
    Dom, Handle, Item, MAX_DEPTH, MAX_TABLES, PieceKind, Sink, TagKind as Kind, TagTok, stand_in,
};

type Builder = TreeBuilder<Handle, Sink>;

/// Parse `source` into a tree with provenance.
pub(super) fn build(source: &str, budget: usize) -> Rc<Dom> {
    let dom = Dom::new();
    dom.set_budget(budget);
    let emitter = Feeder {
        source,
        dom: Rc::clone(&dom),
        tb: TreeBuilder::new(Sink(Rc::clone(&dom)), TreeBuilderOpts::default()),
        position: 0,
        tag_start: 0,
        tag_name: Vec::new(),
        tag_kind: TagKind::EndTag,
        self_closing: false,
        attrs: Vec::new(),
        last_start: Vec::new(),
        comment: Vec::new(),
        doctype: DoctypeParts::default(),
        text: TextState::default(),
        seq: 0,
        depth: DepthCache::default(),
        template_depth: 0,
        skip_foreign: 0,
        skip_integration: 0,
        skip_ftemplates: 0,
        ignore_lf: false,
    };
    for _ in Tokenizer::new_with_emitter(source, emitter) {}
    dom
}

#[derive(Default)]
struct DoctypeParts {
    name: Vec<u8>,
    public: Option<Vec<u8>>,
    system: Option<Vec<u8>>,
    force_quirks: bool,
}

/// What the scan did with a tag.
enum Skip {
    /// It goes to the tree builder.
    Feed,
    /// It never reaches the tree builder, and the tokenizer switches to this state.
    Handled(Option<State>),
}

/// HTML start tags that end SVG and `MathML` content (and `<font>` with an attribute).
const BREAKOUT: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
    "font",
];

/// SVG and `MathML` elements that hold HTML.
const INTEGRATION: &[&str] = &[
    "title",
    "desc",
    "foreignobject",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
];

/// Counts the handles the tree builder holds, and the open `<table>` elements among them.
#[derive(Default)]
struct Count {
    all: std::cell::Cell<usize>,
    tables: std::cell::Cell<usize>,
}

impl Tracer for Count {
    type Handle = Handle;
    fn trace_handle(&self, node: &Handle) {
        self.all.set(self.all.get() + 1);
        if node.is_html_table() {
            self.tables.set(self.tables.get() + 1);
        }
    }
}

/// What the scan knows about the depth of the open elements.
///
/// The tree builder does not say when it pops an element, so the depth is counted from its
/// stack. Every open element was created, so the last count plus the elements created since
/// bounds the depth, and the stack is counted again only when that bound reaches the cap.
#[derive(Default)]
struct DepthCache {
    /// Open elements at the last count.
    verified: usize,
    /// Elements created by then.
    created_at: usize,
    /// Whether the last count was at the cap.
    deep: bool,
    /// Whether a token went to the tree builder since the last count.
    dirty: bool,
}

/// Where the scan is in a run of text.
#[derive(Default)]
struct TextState {
    /// Where the next text byte sits in the source.
    cursor: usize,
    /// The start of a multi-byte character that a chunk ended inside.
    carry: Vec<u8>,
    in_cdata: bool,
    /// Whether the tokenizer is in RCDATA, raw text or script data.
    raw: bool,
}

struct Feeder<'a> {
    source: &'a str,
    dom: Rc<Dom>,
    tb: Builder,
    /// Source bytes consumed so far, as the tokenizer counts them. Reliable at the end of
    /// a tag and of a character reference.
    position: usize,
    tag_start: usize,
    tag_name: Vec<u8>,
    tag_kind: TagKind,
    self_closing: bool,
    attrs: Vec<(Vec<u8>, Vec<u8>)>,
    last_start: Vec<u8>,
    comment: Vec<u8>,
    doctype: DoctypeParts,
    text: TextState,
    /// Count of stand-ins handed out.
    seq: usize,
    depth: DepthCache,
    /// How many `<template>` start tags are open in the part being skipped.
    template_depth: usize,
    /// Open `<svg>` and `<math>` elements in the part being skipped.
    skip_foreign: usize,
    /// Open HTML integration points among them.
    skip_integration: usize,
    /// Open SVG elements named `template` in the part being skipped.
    skip_ftemplates: usize,
    /// Whether the tree builder drops a newline that opens the next text, as it does after
    /// `<pre>`, `<listing>` and `<textarea>`.
    ignore_lf: bool,
}

/// The longest attribute value kept. The longest value the tree builder compares is 21.
const KEPT_VALUE: usize = 32;

/// Whether the tree builder reads attribute `name` on element `element`: `type` on `<input>`,
/// `color`, `face` and `size` on `<font>` (which ends SVG), and `encoding` on
/// `<annotation-xml>`. Nothing else reaches it, and each attribute reaches it only on the
/// element that reads it. So a tag with thousands of attributes costs the same as one with
/// none, and formatting elements stay identical under the spec's limit on repeats.
fn read_by(element: &str, name: &[u8]) -> Option<&'static str> {
    match (element, name) {
        ("input", b"type") => Some("type"),
        ("annotation-xml", b"encoding") => Some("encoding"),
        ("font", b"color") => Some("color"),
        ("font", b"face") => Some("face"),
        ("font", b"size") => Some("size"),
        _ => None,
    }
}

impl Feeder<'_> {
    /// The attributes of the current tag that the tree builder needs, first one wins.
    fn kept_attrs(&self, element: &str) -> Vec<Attribute> {
        let mut kept: Vec<Attribute> = Vec::new();
        for (name, value) in &self.attrs {
            let Some(known) = read_by(element, name) else {
                continue;
            };
            if kept.iter().any(|a| *a.name.local == *known) {
                continue;
            }
            // Only `type` and `encoding` are compared to a value.
            let value = if matches!(known, "type" | "encoding") {
                String::from_utf8_lossy(value)
                    .chars()
                    .take(KEPT_VALUE)
                    .collect()
            } else {
                String::new()
            };
            kept.push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(known)),
                value: StrTendril::from_slice(&value),
            });
        }
        kept
    }

    fn feed(
        &mut self,
        range: std::ops::Range<usize>,
        end_of: Option<String>,
        token: Token,
    ) -> Option<State> {
        if self.dom.exhausted.get() {
            return None;
        }
        self.dom.charge(1);
        self.depth.dirty = true;
        *self.dom.current.borrow_mut() = (range, end_of);
        let mut stats = self.dom.stats.get();
        if let Token::TagToken(tag) = &token {
            stats.marker_tags_fed += usize::from(matches!(
                &*tag.name,
                "template" | "object" | "applet" | "marquee"
            ));
        }
        stats.tokens += 1;
        self.dom.stats.set(stats);
        match self.tb.process_token(token, 0) {
            TokenSinkResult::Plaintext => Some(State::PlainText),
            TokenSinkResult::RawData(kind) => Some(match kind {
                RawKind::Rcdata => State::RcData,
                RawKind::Rawtext => State::RawText,
                RawKind::ScriptData | RawKind::ScriptDataEscaped(_) => State::ScriptData,
            }),
            Sinked::Continue | Sinked::Script(_) | Sinked::EncodingIndicator(_) => None,
        }
    }

    /// An upper bound on the open elements: the last count plus the elements created since.
    fn depth_bound(&self) -> usize {
        self.depth.verified + (self.dom.created.get() - self.depth.created_at)
    }

    /// Whether as many `<table>` elements are open as the cap allows.
    fn too_many_tables(&mut self) -> bool {
        if self.depth_bound() < MAX_TABLES {
            return false;
        }
        let count = Count::default();
        self.tb.trace_handles(&count);
        let mut counts = self.dom.stats.get();
        counts.traced += count.all.get();
        self.dom.stats.set(counts);
        self.dom.charge(count.all.get());
        count.tables.get() >= MAX_TABLES
    }

    /// Track SVG and `MathML` in the skipped part, with the rules that end them.
    fn skip_start(&mut self, local: &str) {
        let foreign = self.skip_foreign > 0 && self.skip_integration == 0;
        if foreign && (BREAKOUT.contains(&local) && (local != "font" || !self.attrs.is_empty())) {
            self.skip_foreign = 0;
            self.skip_integration = 0;
            self.skip_ftemplates = 0;
        }
        let foreign = self.skip_foreign > 0 && self.skip_integration == 0;
        if foreign && INTEGRATION.contains(&local) {
            self.skip_integration += 1;
        }
        if matches!(local, "svg" | "math") {
            self.skip_foreign += 1;
        }
    }

    fn skip_end(&mut self, local: &str) {
        if self.skip_foreign > 0 && self.skip_integration == 0 && matches!(local, "p" | "br") {
            self.skip_foreign = 0;
            self.skip_ftemplates = 0;
        } else if self.skip_integration > 0 && INTEGRATION.contains(&local) {
            self.skip_integration -= 1;
        } else if matches!(local, "svg" | "math") {
            self.skip_foreign = self.skip_foreign.saturating_sub(1);
            if self.skip_foreign == 0 {
                self.skip_integration = 0;
            }
        }
    }

    /// Handle a tag that never reaches the tree builder. Returns the tokenizer state to
    /// switch to when it is one of those.
    ///
    /// - Everything between `<template>` and its end tag is skipped, nesting included. The
    ///   content is never prose, and the markers that a template and its contents leave in
    ///   the tree builder's list of formatting elements cost time that no stack depth shows.
    /// - `<object>`, `<applet>` and `<marquee>` are transparent: their tags are dropped and
    ///   their content is read as part of the flow around them. They insert markers too.
    /// - A `<table>` inside as many tables as the cap allows is dropped, which bounds the
    ///   markers that cells and captions leave.
    fn skipped(&mut self, local: &str, range: &std::ops::Range<usize>) -> Skip {
        let start = self.tag_kind == TagKind::StartTag;
        if self.template_depth > 0 {
            self.dom.charge(1);
            // A `<template>` in SVG is an SVG element, and only a real one nests.
            let plain = !(self.skip_foreign > 0 && self.skip_integration == 0);
            let state = if start {
                self.last_start.clone_from(&self.tag_name);
                if local == "template" {
                    if plain {
                        self.template_depth += 1;
                    } else {
                        self.skip_ftemplates += 1;
                    }
                }
                self.skip_start(local);
                // Nothing builds a tree here, so the tokenizer switches states by name,
                // except inside SVG and `MathML`, where those names are plain elements.
                if self.skip_foreign > 0 && self.skip_integration == 0 {
                    None
                } else {
                    naive_next_state(local.as_bytes())
                }
            } else {
                self.last_start.clear();
                if local == "template" {
                    if self.skip_ftemplates > 0 {
                        self.skip_ftemplates -= 1;
                    } else {
                        // It reaches out of SVG and ends the real template.
                        self.template_depth -= 1;
                        self.skip_foreign = 0;
                        self.skip_integration = 0;
                    }
                }
                self.skip_end(local);
                None
            };
            self.text.raw = state.is_some();
            self.after_markup();
            return Skip::Handled(state);
        }
        let template = start
            && local == "template"
            && !self
                .tb
                .adjusted_current_node_present_but_not_in_html_namespace();
        let transparent = matches!(local, "object" | "applet" | "marquee");
        let table = start && local == "table" && self.too_many_tables();
        if !(template || transparent || table) {
            return Skip::Feed;
        }
        self.dom.charge(1);
        if template {
            self.template_depth = 1;
            self.skip_foreign = 0;
            self.skip_integration = 0;
            self.skip_ftemplates = 0;
        } else {
            self.dom.tags.borrow_mut().push(TagTok {
                range: range.clone(),
                kind: if start {
                    Kind::Start
                } else {
                    Kind::End(local.to_owned())
                },
            });
            if table {
                let mut stats = self.dom.stats.get();
                stats.tables_dropped += 1;
                self.dom.stats.set(stats);
            }
        }
        if start {
            self.last_start.clone_from(&self.tag_name);
        } else {
            self.last_start.clear();
        }
        self.text.raw = false;
        self.after_markup();
        Skip::Handled(None)
    }

    /// Whether the open elements are as deep as the cap allows.
    fn too_deep(&mut self) -> bool {
        let created = self.dom.created.get();
        let bound = self.depth_bound();
        if bound < MAX_DEPTH {
            return false;
        }
        if !self.depth.dirty {
            // Nothing changed since the last count, so it still holds.
            return self.depth.deep;
        }
        let count = Count::default();
        self.tb.trace_handles(&count);
        let visited = count.all.get();
        let mut counts = self.dom.stats.get();
        counts.traced += visited;
        self.dom.stats.set(counts);
        self.dom.charge(visited);
        // The count includes the document and the head and form pointers.
        self.depth.verified = visited.saturating_sub(1);
        self.depth.created_at = created;
        self.depth.deep = self.depth.verified >= MAX_DEPTH;
        self.depth.dirty = false;
        self.depth.deep
    }

    /// A token other than text ends the run of text before it and starts the next one.
    fn after_markup(&mut self) {
        self.ignore_lf = false;
        self.text.cursor = self.position;
        self.text.carry.clear();
        self.text.in_cdata = false;
    }

    /// Feed a run of decoded characters with their source ranges.
    fn text(&mut self, chars: Vec<(char, PieceKind, std::ops::Range<usize>)>) {
        if chars.is_empty() || self.dom.exhausted.get() {
            return;
        }
        self.dom.charge(chars.len());
        let range = chars[0].2.start..chars[chars.len() - 1].2.end;
        let mut run = String::new();
        let mut ignore_lf = std::mem::take(&mut self.ignore_lf);
        for (decoded, kind, src) in chars {
            if decoded == '\0' && kind == PieceKind::Plain {
                self.flush_run(&mut run, &range);
                self.feed(range.clone(), None, Token::NullCharacterToken);
                continue;
            }
            let fed = if decoded.is_ascii_whitespace() && kind == PieceKind::Plain {
                decoded
            } else {
                let fed = stand_in(self.seq);
                self.seq += 1;
                fed
            };
            // The tree builder drops this newline, so it never comes back as inserted text.
            let dropped = std::mem::take(&mut ignore_lf) && decoded == '\n';
            ignore_lf = false;
            if !dropped {
                self.dom.queue.borrow_mut().push_back(Item {
                    fed,
                    kind,
                    decoded,
                    src,
                });
            }
            run.push(fed);
        }
        self.flush_run(&mut run, &range);
    }

    fn flush_run(&mut self, run: &mut String, range: &std::ops::Range<usize>) {
        if run.is_empty() {
            return;
        }
        let text = StrTendril::from_slice(run);
        run.clear();
        self.feed(range.clone(), None, Token::CharacterTokens(text));
    }
}

impl Emitter for Feeder<'_> {
    type Token = ();

    fn set_last_start_tag(&mut self, last_start_tag: Option<&[u8]>) {
        self.last_start = last_start_tag.unwrap_or_default().to_vec();
    }

    fn emit_eof(&mut self) {
        let end = self.source.len();
        self.feed(end..end, None, Token::EOFToken);
        self.tb.end();
    }

    fn emit_error(&mut self, _error: Error) {}

    fn should_emit_errors(&mut self) -> bool {
        false
    }

    fn pop_token(&mut self) -> Option<()> {
        None
    }

    fn emit_string(&mut self, c: &[u8]) {
        if c.is_empty() || self.template_depth > 0 {
            return;
        }
        let bytes = self.source.as_bytes();
        let foreign = self
            .tb
            .adjusted_current_node_present_but_not_in_html_namespace();
        let start = skip_markup(
            bytes,
            self.text.cursor,
            self.text.raw,
            foreign,
            &mut self.text.in_cdata,
        );
        let mut all = std::mem::take(&mut self.text.carry);
        all.extend_from_slice(c);
        let plain = match_plain(bytes, start, &all);
        let entity = bytes.get(start) == Some(&b'&')
            && self.position > start
            && self.position <= bytes.len()
            && plain != Some(self.position - start)
            && self.source.is_char_boundary(self.position);
        if entity {
            let src = start..self.position;
            let text = String::from_utf8_lossy(&all).into_owned();
            self.text.cursor = self.position;
            let chars = text
                .chars()
                .map(|ch| (ch, PieceKind::Entity, src.clone()))
                .collect();
            self.text(chars);
            return;
        }
        // Split off a character that the chunk ended inside.
        let valid = match std::str::from_utf8(&all) {
            Ok(text) => text.len(),
            Err(e) => e.valid_up_to(),
        };
        let tail = all.split_off(valid);
        let Ok(text) = String::from_utf8(all) else {
            return;
        };
        let mut chars = Vec::new();
        let mut at = start;
        if plain.is_some() {
            for ch in text.chars() {
                let len = if ch == '\n' {
                    newline_len(bytes, at)
                } else {
                    ch.len_utf8()
                };
                chars.push((ch, PieceKind::Plain, at..at + len));
                at += len;
            }
            self.text.carry = tail;
        } else {
            // The tokenizer changed the text, as it does for a NUL in RCDATA. A NUL maps to
            // its own byte; anything else has no source of its own.
            let nul = text == "\u{fffd}" && bytes.get(start) == Some(&0);
            for ch in text.chars() {
                let src = if nul { start..start + 1 } else { start..start };
                chars.push((ch, PieceKind::Opaque, src));
            }
            at = if nul { start + 1 } else { start };
        }
        self.text.cursor = at;
        self.text(chars);
    }

    fn init_start_tag(&mut self) {
        self.tag_name.clear();
        self.tag_kind = TagKind::StartTag;
        self.self_closing = false;
        self.attrs.clear();
    }

    fn init_end_tag(&mut self) {
        self.tag_name.clear();
        self.tag_kind = TagKind::EndTag;
        self.attrs.clear();
    }

    fn init_comment(&mut self) {
        self.comment.clear();
    }

    fn start_open_tag(&mut self) {
        self.tag_start = self.position.saturating_sub(1);
    }

    fn emit_current_tag(&mut self) -> Option<State> {
        let range = self.tag_start..self.position.min(self.source.len());
        let local = String::from_utf8_lossy(&self.tag_name).into_owned();
        if let Skip::Handled(state) = self.skipped(&local, &range) {
            return state;
        }
        let local_name = local.clone();
        let mut dropped = false;
        let attrs = self.kept_attrs(&local);
        let mut counts = self.dom.stats.get();
        counts.attrs_passed += attrs.len();
        self.dom.stats.set(counts);
        self.dom.created_html.set(None);
        let state = if self.tag_kind == TagKind::StartTag {
            self.last_start.clone_from(&self.tag_name);
            self.dom.tags.borrow_mut().push(TagTok {
                range: range.clone(),
                kind: Kind::Start,
            });
            if self.too_deep() {
                // Too deep to nest further. Drop the tag, as a browser does.
                let mut stats = self.dom.stats.get();
                stats.dropped_deep += 1;
                self.dom.stats.set(stats);
                dropped = true;
                None
            } else {
                let tag = Tag {
                    kind: TagKind::StartTag,
                    name: LocalName::from(local.as_str()),
                    self_closing: self.self_closing,
                    attrs,
                    had_duplicate_attributes: false,
                };
                self.feed(range, None, Token::TagToken(tag))
            }
        } else {
            self.last_start.clear();
            self.dom.tags.borrow_mut().push(TagTok {
                range: range.clone(),
                kind: Kind::End(local.clone()),
            });
            let tag = Tag {
                kind: TagKind::EndTag,
                name: LocalName::from(local.as_str()),
                self_closing: false,
                attrs: Vec::new(),
                had_duplicate_attributes: false,
            };
            self.feed(range, Some(local), Token::TagToken(tag))
        };
        let created_html = self.dom.created_html.get();
        self.text.raw = state.is_some();
        self.after_markup();
        // The tree builder drops a newline right after these.
        // Only the HTML element does: a `<textarea>` inside SVG is plain SVG.
        self.ignore_lf = self.tag_kind == TagKind::StartTag
            && !dropped
            && created_html == Some(true)
            && matches!(local_name.as_str(), "pre" | "listing" | "textarea");
        state
    }

    fn emit_current_comment(&mut self) {
        if self.template_depth > 0 {
            self.after_markup();
            return;
        }
        let range = self.tag_start..self.position.min(self.source.len());
        self.dom.tags.borrow_mut().push(TagTok {
            range: range.clone(),
            kind: Kind::Comment,
        });
        let text = String::from_utf8_lossy(&self.comment).into_owned();
        self.feed(
            range,
            None,
            Token::CommentToken(StrTendril::from_slice(&text)),
        );
        self.after_markup();
    }

    fn emit_current_doctype(&mut self) {
        if self.template_depth > 0 {
            self.after_markup();
            return;
        }
        let range = self.tag_start..self.position.min(self.source.len());
        let part = |v: &Vec<u8>| StrTendril::from_slice(&String::from_utf8_lossy(v));
        let doctype = Doctype {
            name: Some(part(&self.doctype.name)),
            public_id: self.doctype.public.as_ref().map(part),
            system_id: self.doctype.system.as_ref().map(part),
            force_quirks: self.doctype.force_quirks,
        };
        self.feed(range, None, Token::DoctypeToken(doctype));
        self.after_markup();
    }

    fn set_self_closing(&mut self) {
        self.self_closing = true;
    }

    fn set_force_quirks(&mut self) {
        self.doctype.force_quirks = true;
    }

    fn push_tag_name(&mut self, s: &[u8]) {
        self.tag_name.extend_from_slice(s);
    }

    fn push_comment(&mut self, s: &[u8]) {
        self.comment.extend_from_slice(s);
    }

    fn push_doctype_name(&mut self, s: &[u8]) {
        self.doctype.name.extend_from_slice(s);
    }

    fn init_doctype(&mut self) {
        self.doctype = DoctypeParts::default();
    }

    fn init_attribute(&mut self) {
        self.attrs.push((Vec::new(), Vec::new()));
    }

    fn push_attribute_name(&mut self, s: &[u8]) {
        if let Some(last) = self.attrs.last_mut() {
            last.0.extend_from_slice(s);
        }
    }

    fn push_attribute_value(&mut self, s: &[u8]) {
        if let Some(last) = self.attrs.last_mut() {
            last.1.extend_from_slice(s);
        }
    }

    fn set_doctype_public_identifier(&mut self, value: &[u8]) {
        self.doctype.public = Some(value.to_vec());
    }

    fn set_doctype_system_identifier(&mut self, value: &[u8]) {
        self.doctype.system = Some(value.to_vec());
    }

    fn push_doctype_public_identifier(&mut self, s: &[u8]) {
        self.doctype
            .public
            .get_or_insert_with(Vec::new)
            .extend_from_slice(s);
    }

    fn push_doctype_system_identifier(&mut self, s: &[u8]) {
        self.doctype
            .system
            .get_or_insert_with(Vec::new)
            .extend_from_slice(s);
    }

    fn current_is_appropriate_end_tag_token(&mut self) -> bool {
        self.tag_kind == TagKind::EndTag
            && !self.last_start.is_empty()
            && self.tag_name == self.last_start
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&mut self) -> bool {
        self.tb
            .adjusted_current_node_present_but_not_in_html_namespace()
    }

    fn move_position(&mut self, offset: isize) {
        self.position = self.position.saturating_add_signed(offset);
    }
}

/// Length in the source of the newline at `at`: CR LF, CR or LF.
fn newline_len(source: &[u8], at: usize) -> usize {
    match (source.get(at), source.get(at + 1)) {
        (Some(b'\r'), Some(b'\n')) => 2,
        _ => 1,
    }
}

/// Step over markup that the tokenizer consumes without producing text, so it can sit
/// between two chunks. In data text that is `</>`. In SVG and `MathML` it is also the
/// delimiters of a CDATA section, whose content is text. In RCDATA and raw text `</>` is
/// text.
fn skip_markup(
    source: &[u8],
    mut at: usize,
    raw: bool,
    foreign: bool,
    in_cdata: &mut bool,
) -> usize {
    loop {
        let rest = &source[at.min(source.len())..];
        if !raw && !*in_cdata && rest.starts_with(b"</>") {
            at += 3;
        } else if foreign && !*in_cdata && rest.starts_with(b"<![CDATA[") {
            at += 9;
            *in_cdata = true;
        } else if *in_cdata && rest.starts_with(b"]]>") {
            at += 3;
            *in_cdata = false;
        } else {
            return at;
        }
    }
}

/// How many source bytes at `at` spell `text`, where a newline stands for CR, LF or CR LF.
fn match_plain(source: &[u8], at: usize, text: &[u8]) -> Option<usize> {
    let mut cursor = at;
    for &b in text {
        if b == b'\n' {
            match (source.get(cursor), source.get(cursor + 1)) {
                (Some(b'\r'), Some(b'\n')) => cursor += 2,
                (Some(b'\r' | b'\n'), _) => cursor += 1,
                _ => return None,
            }
        } else if source.get(cursor) == Some(&b) {
            cursor += 1;
        } else {
            return None;
        }
    }
    Some(cursor - at)
}
