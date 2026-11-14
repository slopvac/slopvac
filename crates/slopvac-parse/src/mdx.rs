//! MDX through markdown-rs.

use markdown::mdast::Node;
use markdown::{Constructs, MdxExpressionKind, MdxSignal, ParseOptions, to_mdast};
use slopvac_model::Span;

use crate::align::{ProseBuilder, align};
use crate::code::{code_block, trim_terminator};
use crate::markdown::{MASK, finish, resolve_body};
use crate::mask::{Masked, scan};
use crate::{Block, BlockKind, Document, ParseError, TokenKind};

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
    let masked = scan(&source[body.start()..body.end()], body.start());
    let options = ParseOptions {
        constructs: Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            gfm_task_list_item: true,
            ..Constructs::mdx()
        },
        mdx_expression_parse: Some(Box::new(check_expression)),
        mdx_esm_parse: Some(Box::new(|_| MdxSignal::Ok)),
        ..ParseOptions::mdx()
    };
    let tree = to_mdast(&masked.text, &options).map_err(|m| ParseError::Mdx(m.to_string()))?;
    let mut walker = Walker {
        source,
        masked: &masked,
        blocks: Vec::new(),
        code_spans: Vec::new(),
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

struct Walker<'a> {
    source: &'a str,
    masked: &'a Masked,
    blocks: Vec<Block>,
    code_spans: Vec<Span>,
}

/// Masked byte range of a node.
fn range(node: &Node) -> Option<(usize, usize)> {
    node.position().map(|p| (p.start.offset, p.end.offset))
}

impl Walker<'_> {
    fn span(&self, (start, end): (usize, usize)) -> Span {
        trim_terminator(self.source, self.masked.span(start, end))
    }

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
                    });
                }
                for (row, tr) in table.children.iter().enumerate() {
                    for (column, cell) in tr.children().into_iter().flatten().enumerate() {
                        self.prose(cell, BlockKind::TableCell { row, column }, false);
                    }
                }
            }
            Node::Code(_) => {
                if let Some(r) = range(node) {
                    let span = self.span(r);
                    self.blocks.push(Block {
                        kind: code_block(self.source, span, true),
                        span,
                        projection: None,
                    });
                }
            }
            _ => {
                for child in node.children().into_iter().flatten() {
                    self.block(child);
                }
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
                    && !prose.is_empty()
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
            _ => {}
        }
    }
}

/// Tell markdown-rs whether the text between braces is a complete expression.
///
/// Without a parser the library counts every `{`, so `{'{'}` never closes. This check
/// ignores braces inside string and template literals. It does not validate JavaScript.
fn check_expression(code: &str, _kind: &MdxExpressionKind) -> MdxSignal {
    let mut depth = 0_usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in code.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else {
            match c {
                '"' | '\'' | '`' => quote = Some(c),
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    if quote.is_none() && depth == 0 {
        MdxSignal::Ok
    } else {
        MdxSignal::Eof(
            "Unexpected end of file in expression".into(),
            Box::new("slopvac-parse".into()),
            Box::new("unexpected-eof".into()),
        )
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
    let inner = &expr[1..expr.len() - 1];
    let plain = !inner.contains([quote, '\\', '\n']) && (quote != '`' || !inner.contains("${"));
    plain.then(|| inner.to_owned())
}
