//! MDX through markdown-rs.

use crate::sub;
use markdown::mdast::Node;
use markdown::{Constructs, MdxExpressionParse, MdxSignal, ParseOptions, to_mdast};
use slopvac_model::Span;

use crate::align::{ProseBuilder, align};
use crate::code::{code_kind, mdast_body, trim_terminator};
use crate::jsexpr::ExpressionChecker;
use crate::markdown::{MASK, finish, resolve_body};
use crate::mask::{CodeRegion, Masked, scan};
use crate::{Block, BlockKind, Container, Document, ParseError, TokenKind};

/// Read `text` with the MDX options.
///
/// The brace checker is tried first. If the text does not parse with it, markdown-rs reads
/// it again with plain brace counting, so the checker can only add documents, never lose
/// one that plain counting accepts. When both fail the plain-counting error is returned.
fn parse(text: &str) -> Result<Node, ParseError> {
    read(text, true).or_else(|_| read(text, false))
}

fn read(text: &str, with_checker: bool) -> Result<Node, ParseError> {
    let checker = ExpressionChecker::default();
    let options = ParseOptions {
        constructs: Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            gfm_task_list_item: true,
            ..Constructs::mdx()
        },
        mdx_expression_parse: with_checker.then(|| {
            let parse: Box<MdxExpressionParse> =
                Box::new(move |code: &str, kind: &_| checker.check(code, kind));
            parse
        }),
        mdx_esm_parse: Some(Box::new(|_| MdxSignal::Ok)),
        ..ParseOptions::mdx()
    };
    to_mdast(text, &options).map_err(|m| ParseError::Mdx(m.to_string()))
}

/// Fenced code blocks of a tree, from the parser's own nodes.
fn code_regions(tree: &Node) -> Vec<CodeRegion> {
    fn walk(node: &Node, out: &mut Vec<CodeRegion>) {
        if matches!(node, Node::Code(_))
            && let Some((start, end)) = range(node)
        {
            out.push(CodeRegion {
                range: start..end,
                fenced: true,
            });
        }
        for child in node.children().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

/// Parse the `body` range of MDX `source`.
///
/// `body` is where the caller found the content to read, after any front matter. Every
/// position in the result is a byte offset into `source`. ESM blocks and expressions that
/// are not string literals are not prose.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`, and
/// [`ParseError::Mdx`] when the body is not valid MDX.
pub fn parse_mdx(source: &str, body: Span) -> Result<Document, ParseError> {
    let body = resolve_body(source, body)?;
    let text = sub(source, body.start()..body.end());
    // The first pass reads the unmasked body to find out what is code.
    let regions = code_regions(&parse(text)?);
    let masked = scan(text, body.start(), &regions, false);
    let tree = parse(&masked.text)?;
    let mut walker = Walker {
        source,
        masked: &masked,
        blocks: Vec::new(),
        code_spans: Vec::new(),
        containers: Vec::new(),
        lists: Vec::new(),
    };
    walker.block(&tree);
    let Walker {
        mut blocks,
        code_spans,
        ..
    } = walker;
    blocks.extend(masked.admonitions.iter().cloned());
    Ok(finish(blocks, code_spans))
}

/// The source ranges of MDX syntax that is code, not prose: `{expressions}`, `import` and
/// `export` blocks, and the opening and closing tags of JSX elements with their attributes.
/// The text between the tags is not included.
///
/// `body` is read as in [`parse_mdx`]. Every position is a byte offset into `source`.
///
/// # Errors
/// Returns [`ParseError::Body`] or [`ParseError::Mdx`] in the cases of [`parse_mdx`].
pub fn mdx_syntax_spans(source: &str, body: Span) -> Result<Vec<Span>, ParseError> {
    mdx_syntax_spans_counted(source, body).map(|(spans, _)| spans)
}

/// [`mdx_syntax_spans`] with the number of bytes the tag scanner looked at, so a test can
/// check the work stays linear.
#[doc(hidden)]
pub fn mdx_syntax_spans_counted(
    source: &str,
    body: Span,
) -> Result<(Vec<Span>, usize), ParseError> {
    let body = resolve_body(source, body)?;
    let text = sub(source, body.start()..body.end());
    let regions = code_regions(&parse(text)?);
    let masked = scan(text, body.start(), &regions, false);
    let tree = parse(&masked.text)?;
    let mut spans = Vec::new();
    let mut work = 0;
    collect_syntax(&tree, &masked, &mut spans, &mut work);
    Ok((spans, work))
}

fn collect_syntax(node: &Node, masked: &Masked, out: &mut Vec<Span>, work: &mut usize) {
    match node {
        Node::MdxFlowExpression(_) | Node::MdxTextExpression(_) | Node::MdxjsEsm(_) => {
            if let Some((s, e)) = range(node) {
                out.push(masked.span(s, e));
            }
        }
        Node::MdxJsxFlowElement(_) | Node::MdxJsxTextElement(_) => {
            if let Some((s, e)) = range(node) {
                // When the scanner is unsure, protect up to the first child, which is where
                // markdown-rs says the opening tag ends, or the whole element.
                let open_end = jsx_tag_end(&masked.text, s, e, work)
                    .or_else(|| {
                        node.children()
                            .and_then(|c| c.first())
                            .and_then(range)
                            .map(|(first, _)| first)
                    })
                    .map_or(e, |end| end.clamp(s, e));
                out.push(masked.span(s, open_end));
                // A closing tag is the last `</` in the node when the opening tag did not
                // end the node.
                if open_end < e
                    && let Some(close) = sub(&masked.text, open_end..e).rfind("</")
                {
                    out.push(masked.span(open_end + close, e));
                }
            }
        }
        Node::Root(_)
        | Node::Blockquote(_)
        | Node::FootnoteDefinition(_)
        | Node::List(_)
        | Node::Toml(_)
        | Node::Yaml(_)
        | Node::Break(_)
        | Node::InlineCode(_)
        | Node::InlineMath(_)
        | Node::Delete(_)
        | Node::Emphasis(_)
        | Node::FootnoteReference(_)
        | Node::Html(_)
        | Node::Image(_)
        | Node::ImageReference(_)
        | Node::Link(_)
        | Node::LinkReference(_)
        | Node::Strong(_)
        | Node::Text(_)
        | Node::Code(_)
        | Node::Math(_)
        | Node::Heading(_)
        | Node::Table(_)
        | Node::ThematicBreak(_)
        | Node::TableRow(_)
        | Node::TableCell(_)
        | Node::ListItem(_)
        | Node::Definition(_)
        | Node::Paragraph(_) => {}
    }
    for child in node.children().into_iter().flatten() {
        collect_syntax(child, masked, out, work);
    }
}

/// What the tag scanner is inside, innermost last.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Js {
    /// `{ ... }` holding JavaScript.
    Braces,
    /// A template literal. `${` opens [`Js::Braces`] inside it.
    Template,
}

/// Whether a JavaScript line terminator starts at `i`: LF, CR, U+2028 or U+2029.
fn line_terminator(bytes: &[u8], i: usize) -> bool {
    matches!(bytes.get(i), Some(b'\n' | b'\r'))
        || (bytes.get(i) == Some(&0xE2)
            && bytes.get(i + 1) == Some(&0x80)
            && matches!(bytes.get(i + 2), Some(0xA8 | 0xA9)))
}

/// The end of the JSX tag that starts at `start`: its closing `>`, found outside attribute
/// strings and JavaScript. It reads strings (with escapes), template literals with nested
/// `${}`, and `//` and `/* */` comments, and it never reads past `limit`, the end of the
/// element's own range. `work` counts the bytes looked at.
///
/// It returns `None` when it cannot be sure: a `/` that is not a comment (a regular
/// expression or a division, which cannot be told apart without parsing) or a string,
/// template or comment that does not end. The caller then protects more, never less.
fn jsx_tag_end(text: &str, start: usize, limit: usize, work: &mut usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let limit = limit.min(bytes.len());
    let mut stack: Vec<Js> = Vec::new();
    // An open string: its quote, and whether it is JavaScript (with escapes) or a JSX
    // attribute value (without).
    let mut string: Option<(u8, bool)> = None;
    let mut i = start;
    while i < limit {
        *work += 1;
        let Some(&b) = bytes.get(i) else {
            break;
        };
        let next = bytes.get(i + 1).copied();
        if let Some((quote, script)) = string {
            if script && b == b'\\' {
                i += 1;
            } else if b == quote {
                string = None;
            }
        } else if stack.last() == Some(&Js::Template) {
            match (b, next) {
                (b'\\', _) => i += 1,
                (b'`', _) => {
                    stack.pop();
                }
                (b'$', Some(b'{')) => {
                    stack.push(Js::Braces);
                    i += 1;
                }
                _ => {}
            }
        } else {
            let script = !stack.is_empty();
            match (b, next) {
                (b'/', Some(b'/')) if script => {
                    while i < limit && !line_terminator(bytes, i) {
                        i += 1;
                        *work += 1;
                    }
                    if i >= limit {
                        return None;
                    }
                    // The terminator is a character of its own, so step over it below.
                    i += if bytes.get(i) == Some(&0xE2) { 2 } else { 0 };
                }
                (b'/', Some(b'*')) if script => {
                    i += 2;
                    while i + 1 < limit && bytes.get(i..i + 2) != Some(b"*/".as_slice()) {
                        i += 1;
                        *work += 1;
                    }
                    if i + 1 >= limit {
                        return None;
                    }
                    i += 1;
                }
                (b'/', _) if script => return None,
                (b'"' | b'\'', _) => string = Some((b, script)),
                (b'`', _) if script => stack.push(Js::Template),
                (b'{', _) => stack.push(Js::Braces),
                (b'}', _) => {
                    stack.pop();
                }
                (b'>', _) if stack.is_empty() => return Some(i + 1),
                _ => {}
            }
        }
        i += 1;
    }
    None
}

struct Walker<'a> {
    source: &'a str,
    masked: &'a Masked,
    blocks: Vec<Block>,
    code_spans: Vec<Span>,
    /// The lists and block quotes the walk is inside.
    containers: Vec<Container>,
    /// Whether each enclosing list is ordered.
    lists: Vec<bool>,
}

/// Masked byte range of a node.
fn range(node: &Node) -> Option<(usize, usize)> {
    node.position().map(|p| (p.start.offset, p.end.offset))
}

impl Walker<'_> {
    fn span(&self, (start, end): (usize, usize)) -> Span {
        trim_terminator(self.source, self.masked.span(start, end))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the match names every variant of a foreign enum so that a new variant fails to compile; the arms are one line each"
    )]
    fn block(&mut self, node: &Node) {
        match node {
            Node::Paragraph(_) => self.prose(node, BlockKind::Paragraph, false),
            Node::Heading(h) => self.prose(node, BlockKind::Heading { level: h.depth }, true),
            Node::Table(table) => {
                if let Some(r) = range(node) {
                    let span = self.span(r);
                    self.blocks.push(Block {
                        kind: BlockKind::Table,
                        span,
                        projection: None,
                        containers: self.containers.clone(),
                    });
                }
                for (row, tr) in table.children.iter().enumerate() {
                    for (column, cell) in tr.children().into_iter().flatten().enumerate() {
                        self.prose(cell, BlockKind::TableCell { row, column }, false);
                    }
                }
            }
            Node::MdxFlowExpression(expr) => {
                if let (Some(r), Some(text)) = (range(node), string_literal(&expr.value)) {
                    let (start, end) = r;
                    let mut prose = ProseBuilder::default();
                    prose.push(TokenKind::Opaque, &text, self.masked.span(start, end));
                    if let Some(projection) = prose.finish(false) {
                        self.blocks.push(Block {
                            kind: BlockKind::Paragraph,
                            span: self.span(r),
                            projection: Some(projection),
                            containers: self.containers.clone(),
                        });
                    }
                }
            }
            Node::Code(code) => {
                if let Some(r) = range(node) {
                    let span = self.span(r);
                    let (start, end) = mdast_body(&self.masked.text, r.0, r.1, &code.value);
                    let info = match (&code.lang, &code.meta) {
                        (Some(lang), Some(meta)) => format!("{lang} {meta}"),
                        (Some(lang), None) => lang.clone(),
                        _ => String::new(),
                    };
                    self.blocks.push(Block {
                        kind: code_kind(Some(&info), self.masked.span(start, end)),
                        span,
                        projection: None,
                        containers: self.containers.clone(),
                    });
                }
            }
            Node::Root(_)
            | Node::Blockquote(_)
            | Node::FootnoteDefinition(_)
            | Node::MdxJsxFlowElement(_)
            | Node::List(_)
            | Node::MdxjsEsm(_)
            | Node::Toml(_)
            | Node::Yaml(_)
            | Node::Break(_)
            | Node::InlineCode(_)
            | Node::InlineMath(_)
            | Node::Delete(_)
            | Node::Emphasis(_)
            | Node::MdxTextExpression(_)
            | Node::FootnoteReference(_)
            | Node::Html(_)
            | Node::Image(_)
            | Node::ImageReference(_)
            | Node::MdxJsxTextElement(_)
            | Node::Link(_)
            | Node::LinkReference(_)
            | Node::Strong(_)
            | Node::Text(_)
            | Node::Math(_)
            | Node::ThematicBreak(_)
            | Node::TableRow(_)
            | Node::TableCell(_)
            | Node::ListItem(_)
            | Node::Definition(_) => {
                let (lists, containers) = (self.lists.len(), self.containers.len());
                match node {
                    Node::List(list) => self.lists.push(list.ordered),
                    Node::ListItem(_) => self.containers.push(Container::ListItem {
                        ordered: self.lists.last().copied().unwrap_or(false),
                        depth: self.lists.len(),
                    }),
                    Node::Blockquote(_) => self.containers.push(Container::BlockQuote),
                    Node::Root(_)
                    | Node::FootnoteDefinition(_)
                    | Node::MdxJsxFlowElement(_)
                    | Node::MdxjsEsm(_)
                    | Node::Toml(_)
                    | Node::Yaml(_)
                    | Node::Break(_)
                    | Node::InlineCode(_)
                    | Node::InlineMath(_)
                    | Node::Delete(_)
                    | Node::Emphasis(_)
                    | Node::MdxTextExpression(_)
                    | Node::FootnoteReference(_)
                    | Node::Html(_)
                    | Node::Image(_)
                    | Node::ImageReference(_)
                    | Node::MdxJsxTextElement(_)
                    | Node::Link(_)
                    | Node::LinkReference(_)
                    | Node::Strong(_)
                    | Node::Text(_)
                    | Node::Code(_)
                    | Node::Math(_)
                    | Node::MdxFlowExpression(_)
                    | Node::Heading(_)
                    | Node::Table(_)
                    | Node::ThematicBreak(_)
                    | Node::TableRow(_)
                    | Node::TableCell(_)
                    | Node::Definition(_)
                    | Node::Paragraph(_) => {}
                }
                for child in node.children().into_iter().flatten() {
                    self.block(child);
                }
                self.lists.truncate(lists);
                self.containers.truncate(containers);
            }
        }
    }

    fn prose(&mut self, node: &Node, kind: BlockKind, trim: bool) {
        let Some(r) = range(node) else { return };
        let mut prose = ProseBuilder::default();
        for child in node.children().into_iter().flatten() {
            self.inline(child, &mut prose);
        }
        if let Some(projection) = prose.finish(trim) {
            self.blocks.push(Block {
                kind,
                span: self.span(r),
                projection: Some(projection),
                containers: self.containers.clone(),
            });
        }
    }

    fn inline(&mut self, node: &Node, prose: &mut ProseBuilder) {
        match node {
            Node::Text(text) => {
                if let Some(r) = range(node) {
                    align(prose, self.masked, &self.masked.text, r, &text.value);
                }
            }
            Node::InlineCode(_) => {
                if let Some((s, e)) = range(node) {
                    let span = self.masked.span(s, e);
                    self.code_spans.push(span);
                    prose.push(TokenKind::Masked, MASK, span);
                }
            }
            Node::Break(_) => {
                if let Some((s, e)) = range(node)
                    && prose.has_text()
                {
                    prose.push(TokenKind::Collapse, " ", self.masked.span(s, e));
                }
            }
            Node::MdxTextExpression(expr) => {
                if let (Some((s, e)), Some(text)) = (range(node), string_literal(&expr.value)) {
                    prose.push(TokenKind::Opaque, &text, self.masked.span(s, e));
                }
            }
            Node::Emphasis(_)
            | Node::Strong(_)
            | Node::Delete(_)
            | Node::Link(_)
            | Node::LinkReference(_)
            | Node::MdxJsxTextElement(_) => {
                for child in node.children().into_iter().flatten() {
                    self.inline(child, prose);
                }
            }
            Node::Root(_)
            | Node::Blockquote(_)
            | Node::FootnoteDefinition(_)
            | Node::MdxJsxFlowElement(_)
            | Node::List(_)
            | Node::MdxjsEsm(_)
            | Node::Toml(_)
            | Node::Yaml(_)
            | Node::InlineMath(_)
            | Node::FootnoteReference(_)
            | Node::Html(_)
            | Node::Image(_)
            | Node::ImageReference(_)
            | Node::Code(_)
            | Node::Math(_)
            | Node::MdxFlowExpression(_)
            | Node::Heading(_)
            | Node::Table(_)
            | Node::ThematicBreak(_)
            | Node::TableRow(_)
            | Node::TableCell(_)
            | Node::ListItem(_)
            | Node::Definition(_)
            | Node::Paragraph(_) => {}
        }
    }
}

/// The text a JSX expression renders when it is one string literal.
///
/// Escapes and template substitutions are not evaluated: such expressions are not prose.
fn string_literal(expr: &str) -> Option<String> {
    let expr = expr.trim();
    let quote = expr.chars().next()?;
    if !matches!(quote, '"' | '\'' | '`') || !expr.ends_with(quote) || expr.len() < 2 {
        return None;
    }
    let inner = sub(expr, 1..expr.len() - 1);
    let plain = !inner.contains([quote, '\\', '\n']) && (quote != '`' || !inner.contains("${"));
    plain.then(|| inner.to_owned())
}
