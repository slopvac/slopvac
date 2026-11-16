//! Integration tests for eol.
//! Line endings. Every input in the corpus is parsed with LF, CRLF and lone CR, and the
//! results must match: the same blocks, prose, token kinds and spans, shifted to the
//! converted offsets.
//!
//! The corpus holds the documents used across the other test files and the Python parity
//! fixture. One case is skipped: pulldown-cmark does not read a lone CR as a line ending
//! in fences and quotes, so Markdown is compared for LF and CRLF and only checked to parse
//! with a lone CR.

use slopvac_model::Span;
use slopvac_parse::{Block, BlockKind, Document, parse_markdown, parse_mdx};

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn corpus() -> Vec<String> {
    serde_json::from_str(include_str!("fixtures/eol_corpus.json")).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn parse(source: &str, mdx: bool) -> Option<Document> {
    let body = Span::new(0, source.len()).unwrap();
    if mdx {
        parse_mdx(source, body).ok()
    } else {
        parse_markdown(source, body).ok()
    }
}

/// Maps offsets of the LF text to offsets of the converted text.
struct Shift<'a> {
    lf: &'a str,
    extra: usize,
}

impl Shift<'_> {
    fn at(&self, offset: usize) -> usize {
        offset + self.lf[..offset].matches('\n').count() * self.extra
    }

    fn span(&self, span: Span) -> (usize, usize) {
        (self.at(span.start()), self.at(span.end()))
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
fn describe(block: &Block, shift: &Shift<'_>) -> String {
    let kind = match &block.kind {
        BlockKind::Admonition(a) => format!(
            "admonition {} {:?} {:?} {:?}",
            a.kind,
            a.title,
            a.title_span.map(|s| shift.span(s)),
            a.syntax
        ),
        BlockKind::CodeBlock {
            lang,
            info,
            body,
            fenced,
        } => format!("code {lang:?} {info:?} {:?} {fenced}", shift.span(*body)),
        other => format!("{other:?}"),
    };
    let projection = block.projection.as_ref().map(|p| {
        let tokens: Vec<_> = p
            .tokens()
            .iter()
            .map(|t| (t.kind, t.text.clone(), shift.span(t.src)))
            .collect();
        (p.text().to_owned(), tokens)
    });
    format!("{kind} {:?} {projection:?}", shift.span(block.span))
}

fn summary(doc: &Document, shift: &Shift<'_>) -> (Vec<String>, Vec<(usize, usize)>) {
    (
        doc.blocks.iter().map(|b| describe(b, shift)).collect(),
        doc.code_spans.iter().map(|s| shift.span(*s)).collect(),
    )
}

#[test]
fn every_line_ending_gives_the_same_result() {
    let mut compared = 0;
    for lf in corpus() {
        for mdx in [false, true] {
            let Some(base) = parse(&lf, mdx) else {
                continue;
            };
            let identity = Shift { lf: &lf, extra: 0 };
            let want = summary(&base, &identity);
            for (eol, extra) in [("\r\n", 1), ("\r", 0)] {
                if !mdx && eol == "\r" {
                    assert!(parse(&lf.replace('\n', eol), false).is_some(), "{lf:?}");
                    continue;
                }
                let converted = lf.replace('\n', eol);
                let doc = parse(&converted, mdx)
                    .unwrap_or_else(|| panic!("{eol:?} fails to parse: {lf:?}"));
                let shift = Shift { lf: &lf, extra };
                let got = summary(
                    &doc,
                    &Shift {
                        lf: &converted,
                        extra: 0,
                    },
                );
                let want = if extra == 0 {
                    want.clone()
                } else {
                    summary(&base, &shift)
                };
                assert_eq!(got, want, "mdx={mdx} eol={eol:?} {lf:?}");
                compared += 1;
            }
        }
    }
    assert!(compared > 200, "only {compared} comparisons");
}
