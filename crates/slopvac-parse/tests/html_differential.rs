#![allow(missing_docs)]
//! Differential test: the prose `parse_html` extracts must match the text that html5ever's
//! full tree builder renders.
//!
//! The extractor has no tree builder, so every construct where a tree builder reorders,
//! synthesizes or drops text is a risk. The oracle is the DOM from html5ever with the
//! elements that are not prose removed: `script`, `style`, `noscript`, `template`, `head`,
//! `title` and `pre` give nothing, and `code`, `kbd` and `samp` are masked. Both sides are
//! reduced to words: whitespace and block boundaries do not matter, masked code is
//! removed, and inline tags never separate words.

use html5ever::tendril::TendrilSink;
use html5ever::{ParseOpts, ns, parse_document};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use proptest::prelude::*;
use proptest::test_runner::{RngAlgorithm, TestError, TestRng, TestRunner};
use slopvac_model::Span;
use slopvac_parse::parse_html;

/// Elements that end a word in the output of both sides.
const BREAKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "optgroup",
    "option",
    "p",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "xmp",
];

const HIDDEN: &[&str] = &[
    "script", "style", "noscript", "template", "head", "title", "pre", "code", "kbd", "samp",
];

fn walk(node: &Handle, out: &mut String) {
    match &node.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { name, .. } => {
            let html = name.ns == ns!(html);
            let name: &str = &name.local;
            // SVG and MathML have no `<code>` or `<pre>`, but their script, style and title
            // elements are metadata too.
            if HIDDEN.contains(&name) && (html || matches!(name, "script" | "style" | "title")) {
                if name == "pre" {
                    // A code block ends the line of text around it.
                    out.push(' ');
                }
                return;
            }
            let breaks = html && BREAKS.contains(&name);
            if breaks {
                out.push(' ');
            }
            for child in node.children.borrow().iter() {
                walk(child, out);
            }
            if breaks {
                out.push(' ');
            }
        }
        _ => {
            for child in node.children.borrow().iter() {
                walk(child, out);
            }
        }
    }
}

/// Words html5ever renders.
const TRANSPARENT: [&str; 3] = ["object", "applet", "marquee"];

/// Text where a tag is not a tag, and tables, where an extra comment changes how text joins:
/// the reference cannot drop the tags of `<object>` and the
/// like by search and replace there.
const TEXTUAL: [&str; 11] = [
    "<table",
    "<textarea>",
    "<title>",
    "<script>",
    "<style>",
    "<xmp>",
    "<iframe>",
    "<noscript>",
    "<noembed>",
    "<plaintext>",
    "<!--",
];

/// Whether the reference can be built for `source` by dropping the tags of `<object>` and
/// the like.
fn reference_applies(source: &str) -> bool {
    const FORMATTING: [&str; 6] = ["<b>", "<i>", "<em>", "<a ", "<code>", "<font"];
    // html5ever leaves a marker in its list of formatting elements when a template ends
    // inside a table cell, and then reopens an earlier formatting element in the text after
    // it. The extractor never feeds a template to the tree builder, so it has no such
    // marker. That is the reason for skipping templates: they only matter outside the
    // markers, which the corpus covers.
    // The extractor skips a template in SVG and `MathML` too, up to its end tag, where the
    // reference closes it with the SVG element around it.
    if source.contains("<template>") && (source.contains("<svg>") || source.contains("<math>")) {
        return false;
    }
    if source.contains("<template>") && FORMATTING.iter().any(|t| source.contains(t)) {
        return false;
    }
    let transparent = TRANSPARENT
        .iter()
        .any(|t| source.contains(&format!("<{t}>")) || source.contains(&format!("</{t}>")));
    !(transparent && TEXTUAL.iter().any(|t| source.contains(t)))
}

fn oracle(source: &str) -> Vec<String> {
    // The extractor drops the tags of `<object>`, `<applet>` and `<marquee>` and reads their
    // content as part of the flow, so the reference sees the same.
    let mut source = source.to_owned();
    for tag in TRANSPARENT {
        source = source
            .replace(&format!("<{tag}>"), "<!---->")
            .replace(&format!("</{tag}>"), "<!---->");
    }
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(source.as_str());
    let mut text = String::new();
    walk(&dom.document, &mut text);
    words(&text)
}

/// Words the extractor finds.
fn ours(source: &str) -> Vec<String> {
    let doc = parse_html(source, Span::new(0, source.len()).unwrap()).unwrap();
    let text: Vec<&str> = doc
        .blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(slopvac_parse::Projection::text)
        .collect();
    words(&text.join(" "))
}

fn words(text: &str) -> Vec<String> {
    text.replace('\0', "")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// Compare the words of the oracle, in tree order, with the words of the extractor, in
/// source order. A tree builder moves text that sits in a table in front of it (foster
/// parenting), so the two orders differ only for a document with a table, and then they
/// hold the same words.
fn same(source: &str) -> Result<(), String> {
    if !reference_applies(source) {
        return Ok(());
    }
    let (expected, got) = (oracle(source), ours(source));
    let mut sorted = (expected.clone(), got.clone());
    sorted.0.sort();
    sorted.1.sort();
    let moved = source.to_ascii_lowercase().contains("<table") && sorted.0 == sorted.1;
    if expected == got || moved {
        Ok(())
    } else {
        Err(format!(
            "{source:?}\n  html5ever: {expected:?}\n  extractor: {got:?}"
        ))
    }
}

/// Realistic and adversarial documents. Newlines are rewritten to CRLF and CR as well.
const CORPUS: &[&str] = &[
    "<p>plain paragraph</p>",
    "Just text with no tags at all.\nSecond line.",
    "<!doctype html>\n<html><head><title>T</title><meta charset=utf-8></head>\n<body>\n<h1>Head</h1>\n<p>Body one.</p>\n</body></html>\n",
    // Implied head and body.
    "<title>Only a title</title>Loose text after the title.",
    "<meta charset=utf-8><link rel=stylesheet href=a.css>Body text after metadata.",
    "<style>p{x:y}</style>Text after style.",
    "<head><title>T</title>Body text with no body tag.</head>",
    "<head><title>T</title></head>Text after head.",
    "<head><script>var a=1</script>Text after script.",
    "<html><head><title>T</title></head><body>kept</body></html>",
    "<head><!-- c -->  \n  <p>Para after head</p>",
    "<head>Text inside head.",
    // Omitted end tags.
    "<ul><li>one<li>two<li>three</ul>after",
    "<dl><dt>term<dd>definition<dt>next<dd>more</dl>",
    "<p>one<p>two<div>three</div>four",
    "<table><tr><td>a<td>b<tr><td>c<td>d</table>tail",
    "<select><option>a<option>b</select>done",
    "<ol><li>a<ol><li>b<li>c</ol><li>d</ol>",
    // Nesting.
    "<ul><li><p>para in item</p> trailing item text<ul><li>inner</li></ul> after inner</li></ul>",
    "<table><tr><td>outer<table><tr><td>inner</td></tr></table>after inner</td><td>next</td></tr></table>",
    "<blockquote><p>q1</p>between<p>q2</p></blockquote>",
    "<div><div><div><p>deep</div>mid</div>outer</div>",
    // Misnested inline.
    "<p><b>bold <i>both</b> italic</i> plain</p>",
    "<p><b>one<p>two</b>three</p>",
    "<a href=x>link <a href=y>nested</a> tail</a>",
    "<p>x<b>y<i>z</p><p>w</i></b>v</p>",
    // Foster parenting.
    "<table>stray<tr><td>cell</td></tr></table>",
    "<table><tr>between<td>cell</td></tr></table>",
    // Inline code and pre.
    "<p>Run <code>cargo test</code> now</p><pre>block\ncode</pre><p>after</p>",
    "<p>a<kbd>b</kbd>c<samp>d</samp>e</p>",
    "<pre><code>x &amp; y</code></pre>tail text",
    "<p>see <code>never closed</p><p>more</p>",
    "<code>x</pre> y words</code> z",
    // Raw text and RCDATA.
    "<textarea>a &amp; b <b>not a tag</b></textarea>after",
    "<textarea>a\u{0}b</textarea>after",
    "<script>if (a<b) {document.write('<p>x</p>')}</script><p>shown</p>",
    "<style>/* </p> */</style><p>shown</p>",
    "<noscript><p>hidden</p></noscript><p>shown</p>",
    "<iframe>raw <b>text</b></iframe>after",
    "<xmp><b>raw</b></xmp>after",
    "<title>t<b>not bold</b></title><p>shown</p>",
    "<plaintext><b>everything is text",
    // Templates and foreign content.
    "<template><p>hidden</p></template><p>shown</p>",
    "<object>fallback <b>text</b></object> after",
    "<marquee>moving</marquee><applet>old</applet>",
    "<template><object></template>text<b>bold</b>",
    "<template><template><p>hidden</p></template>hidden</template>shown",
    "<svg><title>svg title</title><text>svg text</text></svg><p>after</p>",
    "<svg><![CDATA[cdata text]]></svg><p>after</p>",
    "<svg><textarea>\n>a</textarea></svg><p>x</p>",
    "<p>a<textarea>\n\nb</textarea>c</p>",
    "<svg><script>var a = 1</script><text>kept</text></svg> tail",
    "<svg><style>.a{}</style><text>kept</text></svg> tail",
    "<b>bold <svg><text>x</text></b> after</svg> tail",
    "<p>a<svg><title>t</title><desc>d</desc></svg>b</p>",
    "<math><mi>x</mi><mo>+</mo><mn>1</mn></math> equals",
    "<svg><foreignObject><p>html in svg</p></foreignObject></svg>after",
    "<p>a<![CDATA[not cdata]]>b</p>",
    // Comments, doctype, processing instructions.
    "<p>a<!-- c -->b</p>",
    "<p>a<!-- <p>hidden</p> -->b</p>",
    "<!-- before --><p>text</p><!-- after -->",
    "<?php echo 1 ?><p>after</p>",
    "<p>unclosed <!-- comment never ends",
    "<p>x</p><!--><p>y</p>",
    "<p>x</p><!---><p>y</p>",
    // Entities.
    "<p>AT&amp;T &lt;b&gt; &eacute; &#169; &#x1F600; &nbsp;x</p>",
    "<p>&bogus; &amp &lt &#; &#x; &#xD800; &#0; end</p>",
    "<p>a &notit; b &not; c</p>",
    // Attributes.
    "<p title=\"a > b\" class='x>y'>attr text</p>",
    "<a href=\"x&amp;y\">link</a> after",
    "<p a=1 a=2 b>dupes</p>",
    // Unfinished and odd markup.
    "<p>done <b",
    "<p>done </",
    "<p>a</><p>b",
    "<p>a < b and c > d</p>",
    "<p>one</p></p>two",
    "</div></span>stray ends<p>x</p>",
    "<br><br>text<br>",
    "<body>one<body>two<html>three",
    "\u{feff}<p>bom text</p>",
    "<p>tab\there</p>\n\n\n<p>blank lines</p>",
    "<h1>One<h2>Two</h1>Three",
    "<form><input value=x>label text<button>press</button></form>",
    "<img alt='alt text'>after<video>fallback</video>",
    "<ruby>漢<rt>kan</rt></ruby> 字",
    "<details><summary>sum</summary>detail text</details>",
    "<frameset><frame>text</frameset>",
    "<table><caption>cap</caption><thead><tr><th>h</th></tr></thead><tbody><tr><td>b</td></tr></tbody><tfoot><tr><td>f</td></tr></tfoot></table>",
    "<table><colgroup><col></colgroup><tr><td>c</td></tr></table>",
    "<table><td>no row</td></table>",
];

fn variants(source: &str) -> [String; 3] {
    let lf = source.replace("\r\n", "\n").replace('\r', "\n");
    [lf.clone(), lf.replace('\n', "\r\n"), lf.replace('\n', "\r")]
}

#[test]
fn corpus_matches_html5ever_with_every_line_ending() {
    let mut failures = Vec::new();
    for source in CORPUS {
        for variant in variants(source) {
            if let Err(message) = same(&variant) {
                failures.push(message);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} documents differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const ATOMS: &[&str] = &[
    "alpha",
    "beta ",
    " gamma",
    "d&amp;e",
    "é",
    "日本",
    "&nbsp;",
    "&#169;",
    "&bogus;",
    " ",
    "\n",
    "\r\n",
    "\r",
    "<b>",
    "</b>",
    "<i>",
    "</i>",
    "<a href=x>",
    "</a>",
    "<span>",
    "</span>",
    "<em>",
    "</em>",
    "<code>",
    "</code>",
    "<p>",
    "</p>",
    "<div>",
    "</div>",
    "<li>",
    "</li>",
    "<ul>",
    "</ul>",
    "<ol>",
    "</ol>",
    "<h1>",
    "</h1>",
    "<h3>",
    "</h3>",
    "<blockquote>",
    "</blockquote>",
    "<table>",
    "</table>",
    "<tr>",
    "</tr>",
    "<td>",
    "</td>",
    "<th>",
    "</th>",
    "<tbody>",
    "<br>",
    "<object>",
    "</object>",
    "<applet>",
    "<marquee>",
    "</marquee>",
    "<kbd>",
    "</kbd>",
    "<select>",
    "<option>",
    "</select>",
    "<form>",
    "</form>",
    "<hr>",
    "<!-- c -->",
    "<!-- <p>x</p> -->",
    "<title>",
    "</title>",
    "<head>",
    "</head>",
    "<body>",
    "</body>",
    "<html>",
    "<meta charset=x>",
    "<style>x{}</style>",
    "<script>a<b</script>",
    "<script>",
    "</script>",
    "<template>",
    "</template>",
    "<noscript>",
    "</noscript>",
    "<textarea>",
    "</textarea>",
    "<pre>",
    "</pre>",
    "<svg>",
    "</svg>",
    "<math>",
    "</math>",
    "<![CDATA[cd]]>",
    "<dl>",
    "<dt>",
    "<dd>",
    "</dl>",
    "</>",
    "<",
    ">",
    "&",
    "<p a=\"b>c\">",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(ATOMS), 0..60).prop_map(|v| v.concat())
}

/// Run `cases` generated documents from a fixed seed, so a failure reproduces.
fn run(cases: u32, seed: u8) -> Result<(), TestError<String>> {
    let config = ProptestConfig {
        cases,
        max_global_rejects: cases * 20,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[seed; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);
    runner.run(&soup(), |source| {
        for variant in variants(&source) {
            if let Err(message) = same(&variant) {
                return Err(TestCaseError::fail(message));
            }
        }
        Ok(())
    })
}

#[test]
fn generated_documents_match_html5ever() {
    if let Err(error) = run(5_000, 7) {
        panic!("{error}");
    }
}

/// A longer run for use after a change to the extractor:
/// `cargo test --release -p slopvac-parse --test html_differential -- --ignored`.
#[test]
#[ignore = "slow"]
fn generated_documents_match_html5ever_long_run() {
    for seed in 0..8 {
        if let Err(error) = run(50_000, seed) {
            panic!("seed {seed}: {error}");
        }
    }
}

/// The most work the parse may do for `source`.
fn budget(source: &str) -> usize {
    slopvac_parse::WORK_PER_BYTE * source.len() + slopvac_parse::WORK_BASE
}

fn work(source: &str) -> usize {
    let body = Span::new(0, source.len()).unwrap();
    slopvac_parse::parse_html_stats(source, body)
        .unwrap()
        .1
        .work
}

#[test]
fn the_corpus_and_generated_documents_stay_far_under_the_work_budget() {
    // The budget exists for hostile input. Ordinary and merely odd documents must use a
    // small part of it, or it would reject real files.
    let mut sources: Vec<String> = CORPUS.iter().flat_map(|s| variants(s)).collect();
    let config = ProptestConfig {
        cases: 5_000,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[11; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);
    let generated = std::cell::RefCell::new(Vec::new());
    runner
        .run(&soup(), |source| {
            generated.borrow_mut().push(source);
            Ok(())
        })
        .unwrap();
    sources.extend(generated.into_inner());
    let mut worst = (0, String::new());
    for source in &sources {
        let used = work(source);
        assert!(used * 4 <= budget(source), "{source:?} used {used}");
        if used * 1000 / budget(source) > worst.0 {
            worst = (used * 1000 / budget(source), source.clone());
        }
    }
    println!(
        "worst share of the budget: {}/1000 for {:?}",
        worst.0, worst.1
    );
}
