//! The tree html5ever builds, in an arena, with the source bytes of every text run.
//!
//! The tree builder decides where text and elements go, including the cases a stream
//! parser cannot follow: implied elements, foster parenting, the adoption agency
//! algorithm and template contents. This sink records what it does, and tags each
//! character of text with the source range it came from at the moment of insertion.
//!
//! The tree builder sees one private-use character for each source character that is not
//! ASCII whitespace. When it inserts text, the sink reads those characters back and swaps
//! each for the item the scan queued for it, which carries the source range. Characters
//! the tree builder drops never come back, and the queue skips their items.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;

use html5ever::tendril::StrTendril;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, ExpandedName, QualName, local_name, ns};

/// How a piece of text relates to its source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PieceKind {
    /// The source characters themselves. The text is the source slice.
    Plain,
    /// A decoded character reference. Maps to the whole reference.
    Entity,
    /// Text the tokenizer produced that is not in the source as written.
    Opaque,
}

/// A run of text and the source bytes it came from.
#[derive(Debug, Clone)]
pub(super) struct Seg {
    pub(super) kind: PieceKind,
    /// The decoded text. Empty for [`PieceKind::Plain`], whose text is the source slice.
    pub(super) text: String,
    pub(super) src: Range<usize>,
}

/// One source character fed to the tree builder, waiting to come back as inserted text.
#[derive(Debug)]
pub(super) struct Item {
    /// What the tree builder saw.
    pub(super) fed: char,
    pub(super) kind: PieceKind,
    /// The character as the tokenizer decoded it.
    pub(super) decoded: char,
    pub(super) src: Range<usize>,
}

/// First private-use character the scan hands to the tree builder.
const ID_BASE: u32 = 0xF_0000;
/// Number of distinct stand-in characters. Plane 15 minus its last two code points.
const ID_COUNT: u32 = 0xFFFE;

/// The stand-in for the `n`th non-whitespace source character.
pub(super) fn stand_in(n: usize) -> char {
    let offset = u32::try_from(n % ID_COUNT as usize).expect("below 65534");
    char::from_u32(ID_BASE + offset).expect("plane 15 is valid")
}

fn is_stand_in(c: char) -> bool {
    (ID_BASE..ID_BASE + ID_COUNT).contains(&(c as u32))
}

/// What an element is, besides its name.
#[derive(Debug)]
pub(super) struct Elem {
    pub(super) name: Rc<QualName>,
    /// The start tag. Implied elements get the token that made them.
    pub(super) start: Range<usize>,
    /// The contents of a `<template>`, as a document fragment.
    pub(super) template: Option<usize>,
    /// Whether this is a `MathML` `annotation-xml` that holds HTML.
    annotation_xml: bool,
}

#[derive(Debug)]
pub(super) enum Kind {
    /// The document, or the fragment of a template.
    Root,
    Element(Elem),
    Text(Vec<Seg>),
    /// A comment, doctype or processing instruction.
    Other,
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) kind: Kind,
    pub(super) parent: Option<usize>,
    pub(super) first: Option<usize>,
    pub(super) last: Option<usize>,
    pub(super) prev: Option<usize>,
    pub(super) next: Option<usize>,
}

/// A node of the tree, by index. The name lets the tree builder read an element's name
/// through a reference.
#[derive(Clone, Debug)]
pub(super) struct Handle {
    pub(super) id: usize,
    name: Rc<QualName>,
}

/// A tag, comment or doctype token.
#[derive(Debug, Clone)]
pub(super) struct TagTok {
    pub(super) range: Range<usize>,
    pub(super) kind: TagKind,
}

#[derive(Debug, Clone)]
pub(super) enum TagKind {
    Start,
    /// An end tag, with its local name.
    End(String),
    Comment,
}

impl Handle {
    /// Whether this is an HTML `<table>` element.
    pub(super) fn is_html_table(&self) -> bool {
        self.name.ns == ns!(html) && &*self.name.local == "table"
    }
}

/// Counters for the work the tree builder did, so a test can bound it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    /// Tokens handed to the tree builder.
    pub tokens: usize,
    /// Calls the tree builder made on the sink to change the tree.
    pub tree_ops: usize,
    /// Nodes in the tree.
    pub nodes: usize,
    /// Start tags dropped because the tree was too deep.
    pub dropped_deep: usize,
    /// Open elements visited while counting how deep the tree is.
    pub traced: usize,
    /// Units of work in all: tokens, tree changes, queue steps, stack counts, end tag
    /// scans and tree visits. The budget limits this.
    pub work: usize,
    /// Tags the tree builder received that must never reach it: `<template>`, `<object>`,
    /// `<applet>` and `<marquee>`, which insert markers into its list of formatting elements.
    pub marker_tags_fed: usize,
    /// `<table>` elements the tree builder created.
    pub tables: usize,
    /// `<table>` start tags dropped because as many tables were open as the cap allows.
    pub tables_dropped: usize,
    /// Attributes handed to the tree builder, over all tags.
    pub attrs_passed: usize,
    /// Tags looked at while finding where skipped elements end.
    pub end_scan: usize,
}

/// Open elements nest at most this deep. Browsers cap depth too (Blink stops at 512), and
/// the spec's scope checks cost time in proportion to it.
pub(super) const MAX_DEPTH: usize = 512;

/// Open `<table>` elements nest at most this deep. Deeper `<table>` start tags are dropped.
///
/// The cap, the depth cap and the input cap together bound the time the tree builder can
/// spend, in terms nobody has to measure. Each token costs at most one walk over the stack of
/// open elements (scope checks, at most `MAX_DEPTH`) and one walk over the list of active
/// formatting elements. That list holds markers and formatting elements. The scan never lets
/// `<template>`, `<object>`, `<applet>` or `<marquee>` reach the tree builder, so the only
/// markers come from `<td>`, `<th>` and `<caption>`. An end tag or an implied end for the
/// cell clears the list back to its marker, so at most one marker per open table is live,
/// `MAX_TABLES` in all, which makes `MAX_TABLES + 1` segments. Within a segment the spec keeps
/// at most three formatting elements that are alike in name and attributes. Fourteen names
/// qualify, and the scan keeps only `color`, `face` and `size` on `<font>`, which gives 13
/// plain names and 8 kinds of `<font>`. That is 21 kinds, so 63 entries a segment and
/// `63 * (MAX_TABLES + 1)` = 1,071 for the whole list. So one token costs at most
/// 512 + 1,071 steps, and the longest input (`MAX_HTML_BYTES` at 3 bytes a token, about
/// 5.6 million tokens) at most 9e9 of them. No input reaches that: the work budget refuses
/// anything past 32 units a byte long before. The `html_robustness` benchmark times the worst
/// shapes at the cap.
pub(super) const MAX_TABLES: usize = 16;

/// The tree, and the state the scan and the sink share.
pub(super) struct Dom {
    pub(super) nodes: RefCell<Vec<Node>>,
    pub(super) queue: RefCell<VecDeque<Item>>,
    /// The source range of the token being processed, and whether it is an end tag with
    /// that local name.
    pub(super) current: RefCell<(Range<usize>, Option<String>)>,
    pub(super) stats: Cell<Stats>,
    /// Every tag, comment and doctype in the source, in order. Nothing in the tree says
    /// where an element's end tag was, so the walk reads it from here.
    pub(super) tags: RefCell<Vec<TagTok>>,
    /// Elements the tree builder created. Every open element was created, so the count of
    /// created elements bounds how much the open stack can have grown.
    pub(super) created: Cell<usize>,
    /// Work done so far, in units of one step of anything that could repeat per token.
    work: Cell<usize>,
    budget: Cell<usize>,
    /// Whether the work passed the budget. The scan stops feeding the tree builder.
    pub(super) exhausted: Cell<bool>,
    /// Whether the last element the tree builder created is an HTML element.
    pub(super) created_html: Cell<Option<bool>>,
    unnamed: Rc<QualName>,
    pub(super) document: usize,
}

impl Dom {
    pub(super) fn new() -> Rc<Self> {
        let unnamed = Rc::new(QualName::new(None, ns!(), local_name!("")));
        let root = Node {
            kind: Kind::Root,
            parent: None,
            first: None,
            last: None,
            prev: None,
            next: None,
        };
        Rc::new(Self {
            nodes: RefCell::new(vec![root]),
            queue: RefCell::new(VecDeque::new()),
            current: RefCell::new((0..0, None)),
            stats: Cell::new(Stats::default()),
            tags: RefCell::new(Vec::new()),
            created: Cell::new(0),
            work: Cell::new(0),
            budget: Cell::new(usize::MAX),
            exhausted: Cell::new(false),
            created_html: Cell::new(None),
            unnamed,
            document: 0,
        })
    }

    /// Set the most work the parse may do.
    pub(super) fn set_budget(&self, budget: usize) {
        self.budget.set(budget);
    }

    pub(super) fn work_done(&self) -> usize {
        self.work.get()
    }

    /// Count `units` of work. Passing the budget stops the parse.
    pub(super) fn charge(&self, units: usize) {
        let work = self.work.get().saturating_add(units);
        self.work.set(work);
        if work > self.budget.get() {
            self.exhausted.set(true);
        }
    }

    fn bump(&self, f: impl FnOnce(&mut Stats)) {
        let mut stats = self.stats.get();
        f(&mut stats);
        self.stats.set(stats);
    }

    fn handle(&self, id: usize) -> Handle {
        let nodes = self.nodes.borrow();
        let name = match &nodes[id].kind {
            Kind::Element(e) => Rc::clone(&e.name),
            _ => Rc::clone(&self.unnamed),
        };
        Handle { id, name }
    }

    fn push(&self, kind: Kind) -> usize {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node {
            kind,
            parent: None,
            first: None,
            last: None,
            prev: None,
            next: None,
        });
        nodes.len() - 1
    }

    /// Take the segments for text the tree builder inserted.
    fn segments(&self, text: &str) -> Vec<Seg> {
        let range = self.current.borrow().0.clone();
        let mut out: Vec<Seg> = Vec::new();
        let mut queue = self.queue.borrow_mut();
        for c in text.chars() {
            let item = if is_stand_in(c) || c.is_ascii_whitespace() {
                // Skip what the tree builder dropped.
                let mut found = None;
                while let Some(item) = queue.pop_front() {
                    self.charge(1);
                    if item.fed == c {
                        found = Some(item);
                        break;
                    }
                }
                found
            } else {
                None
            };
            let seg = match item {
                Some(item) if item.kind == PieceKind::Plain => Seg {
                    kind: PieceKind::Plain,
                    text: String::new(),
                    src: item.src,
                },
                Some(item) => Seg {
                    kind: item.kind,
                    text: item.decoded.to_string(),
                    src: item.src,
                },
                // The tree builder made this character up, as it does for a NUL in
                // foreign content. It has no source.
                None => Seg {
                    kind: PieceKind::Opaque,
                    text: c.to_string(),
                    src: range.end..range.end,
                },
            };
            push_seg(&mut out, seg);
        }
        out
    }

    fn detach(&self, id: usize) {
        let mut nodes = self.nodes.borrow_mut();
        let (parent, prev, next) = (nodes[id].parent, nodes[id].prev, nodes[id].next);
        if let Some(p) = prev {
            nodes[p].next = next;
        } else if let Some(par) = parent {
            nodes[par].first = next;
        }
        if let Some(n) = next {
            nodes[n].prev = prev;
        } else if let Some(par) = parent {
            nodes[par].last = prev;
        }
        nodes[id].parent = None;
        nodes[id].prev = None;
        nodes[id].next = None;
    }

    /// Link `id` as the last child of `parent`.
    fn link_last(&self, parent: usize, id: usize) {
        let mut nodes = self.nodes.borrow_mut();
        let last = nodes[parent].last;
        nodes[id].parent = Some(parent);
        nodes[id].prev = last;
        nodes[id].next = None;
        match last {
            Some(l) => nodes[l].next = Some(id),
            None => nodes[parent].first = Some(id),
        }
        nodes[parent].last = Some(id);
    }

    /// Link `id` just before `sibling`.
    fn link_before(&self, sibling: usize, id: usize) {
        let mut nodes = self.nodes.borrow_mut();
        let parent = nodes[sibling].parent;
        let prev = nodes[sibling].prev;
        nodes[id].parent = parent;
        nodes[id].prev = prev;
        nodes[id].next = Some(sibling);
        nodes[sibling].prev = Some(id);
        match prev {
            Some(p) => nodes[p].next = Some(id),
            None => {
                if let Some(par) = parent {
                    nodes[par].first = Some(id);
                }
            }
        }
    }

    fn add_text(&self, segs: Vec<Seg>, place: Place) {
        if segs.is_empty() {
            return;
        }
        // Text joins an adjacent text node.
        let neighbour = {
            let nodes = self.nodes.borrow();
            match place {
                Place::Last(parent) => nodes[parent].last,
                Place::Before(sibling) => nodes[sibling].prev,
            }
        };
        if let Some(n) = neighbour {
            let mut nodes = self.nodes.borrow_mut();
            if let Kind::Text(existing) = &mut nodes[n].kind {
                for seg in segs {
                    push_seg(existing, seg);
                }
                return;
            }
        }
        let id = self.push(Kind::Text(segs));
        match place {
            Place::Last(parent) => self.link_last(parent, id),
            Place::Before(sibling) => self.link_before(sibling, id),
        }
        self.bump(|s| s.nodes += 1);
    }

    /// Children of a node, in order.
    pub(super) fn children(&self, id: usize) -> Vec<usize> {
        let nodes = self.nodes.borrow();
        let mut out = Vec::new();
        let mut at = nodes[id].first;
        while let Some(n) = at {
            out.push(n);
            at = nodes[n].next;
        }
        out
    }
}

#[derive(Clone, Copy)]
enum Place {
    Last(usize),
    Before(usize),
}

/// Add a segment, joining it to the last one when they continue each other.
pub(super) fn push_seg(segs: &mut Vec<Seg>, seg: Seg) {
    if let Some(last) = segs.last_mut()
        && last.kind == seg.kind
    {
        match seg.kind {
            PieceKind::Plain if last.src.end == seg.src.start => {
                last.src.end = seg.src.end;
                return;
            }
            PieceKind::Entity | PieceKind::Opaque if last.src == seg.src => {
                last.text.push_str(&seg.text);
                return;
            }
            _ => {}
        }
    }
    segs.push(seg);
}

/// The sink the tree builder writes to.
pub(super) struct Sink(pub(super) Rc<Dom>);

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = ();
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) {}

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        self.0.handle(self.0.document)
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        target.name.expanded()
    }

    fn create_element(
        &self,
        name: QualName,
        _attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Handle {
        let dom = &self.0;
        dom.bump(|s| {
            s.tree_ops += 1;
            s.tables += usize::from(name.ns == ns!(html) && &*name.local == "table");
        });
        dom.charge(1);
        dom.created.set(dom.created.get() + 1);
        let start = dom.current.borrow().0.clone();
        dom.created_html.set(Some(name.ns == ns!(html)));
        let template = flags.template.then(|| dom.push(Kind::Root));
        let id = dom.push(Kind::Element(Elem {
            name: Rc::new(name),
            start,
            template,
            annotation_xml: flags.mathml_annotation_xml_integration_point,
        }));
        dom.bump(|s| s.nodes += 1);
        dom.handle(id)
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        let id = self.0.push(Kind::Other);
        self.0.handle(id)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        let id = self.0.push(Kind::Other);
        self.0.handle(id)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        let dom = &self.0;
        dom.bump(|s| s.tree_ops += 1);
        dom.charge(1);
        match child {
            NodeOrText::AppendNode(node) => {
                dom.link_last(parent.id, node.id);
            }
            NodeOrText::AppendText(text) => {
                let segs = dom.segments(&text);
                dom.add_text(segs, Place::Last(parent.id));
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = self.0.nodes.borrow()[element.id].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &Handle) -> Handle {
        let nodes = self.0.nodes.borrow();
        let Kind::Element(e) = &nodes[target.id].kind else {
            unreachable!("only a template asks for contents")
        };
        let id = e.template.expect("a template has contents");
        drop(nodes);
        self.0.handle(id)
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let dom = &self.0;
        dom.bump(|s| s.tree_ops += 1);
        dom.charge(1);
        match new_node {
            NodeOrText::AppendNode(node) => {
                dom.link_before(sibling.id, node.id);
            }
            NodeOrText::AppendText(text) => {
                let segs = dom.segments(&text);
                dom.add_text(segs, Place::Before(sibling.id));
            }
        }
    }

    fn add_attrs_if_missing(&self, _target: &Handle, _attrs: Vec<Attribute>) {}

    fn remove_from_parent(&self, target: &Handle) {
        self.0.bump(|s| s.tree_ops += 1);
        self.0.charge(1);
        self.0.detach(target.id);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let dom = &self.0;
        let kids = dom.children(node.id);
        dom.bump(|s| s.tree_ops += kids.len() + 1);
        dom.charge(kids.len() + 1);
        for kid in kids {
            dom.detach(kid);
            dom.link_last(new_parent.id, kid);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &Handle) -> bool {
        matches!(&self.0.nodes.borrow()[handle.id].kind, Kind::Element(e) if e.annotation_xml)
    }
}
