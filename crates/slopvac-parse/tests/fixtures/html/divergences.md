# Divergences from the Python reference

`slopvac-parse` reads HTML with html5ever's tree builder, which follows the WHATWG tree construction rules. A WHATWG tokenizer (html5gum) supplies the tokens and their source positions. The Python projection
(`slopvac.analyze._parse_html`) uses `html.parser` and builds one block per source line.
`reference.json` holds documents where the two agree on every word and on the source text of
every word. The golden suite (slopvac-8l4.2.7) should expect the differences below.

| Area | Python | Rust | Why |
| --- | --- | --- | --- |
| Words split by an inline tag | A synthetic space at every tag, so `un<b>believ</b>able` is three words | One word, `unbelievable`. The tags fall inside its span | Inline markup does not break a word in a browser. The vocabulary rule must see the word that Vale sees. |
| Comments between characters | A synthetic space, so `a<!-- x -->b` is two words | `ab` | A comment is not rendered. |
| Block structure | One paragraph per source line of visible text | Paragraph, heading, list item and table cell blocks, split at block-level tags | The brief asks for blocks. A block spans several lines. |
| `<br>` | Synthetic space between words | A `wrap` token: a synthetic space with an empty source range at the end of the previous text | The position contract. |
| Whitespace | Characters kept one for one; leading and trailing space trimmed per line | Each run is one `collapse` token that covers the whole run, newlines included; the ends of a block are trimmed | The position contract. |
| Entities | Decoded by `html.unescape`; `&bogus;` stays | Decoded by the tokenizer to the WHATWG rules. Same results on every reference in the fixture | Same rules in practice. |
| Entity that decodes to a space (`&#32;`) | A space character | Part of a `collapse` run | Browsers collapse it. `&nbsp;` stays a character in Rust and is whitespace to a word split in both. |
| Byte order mark | Visible text, U+FEFF | Not text. Offsets still count its three bytes | The position contract. |
| Self-closing `<code/>`, `<script/>`, `<pre/>` | Opens and closes at once | Opens the element, as the HTML spec says for a non-void element | The tree builder ignores the slash. |
| Misnested and stray tags | Any skipped end tag ends the innermost skipped element, and a stray end tag splits the line | The tree builder decides: implied end tags, scope rules, the adoption agency algorithm, text that a table moves, and template contents all follow the spec | The brief asks for blocks, and the html5ever differential test checks them. |
| `<head>` without `</head>` | Skips the rest of the document | The head ends where the spec ends it, at the first element or text that does not belong in it | Same. |
| Inline `<code>`, `<kbd>`, `<samp>` | Dropped | A `masked` token with the text NUL, and an entry in `Document::code_spans` | Same as masked code in Markdown. |
| `<pre>` | Dropped | A `CodeBlock` with the body span | Code is a block that the metrics count. |
| `<title>` | Visible outside `<head>` | Never prose, in HTML or SVG | A title is metadata wherever it sits. |
| `<textarea>` | Visible | Visible | No difference. |
| Front matter | Not detected | Not detected; the caller passes the body range | Front matter is slopvac-8l4.3.4. |
| Markdown fences inside HTML | `_html_fence_lines` blanks them when HTML sits in a Markdown block | Not handled. `parse_html` reads a whole HTML document | Reading the HTML blocks of a Markdown file is a Markdown parser concern. |
| Table containers | None | Cells are blocks. The table itself is not a block | The cells carry the prose. |
| SVG and MathML | Plain tags | Elements there are not HTML: `<code>` and `<pre>` are not code, a CDATA section is text, and the tree builder's breakout rules apply | Same. |
| Nesting | No limit | At most 512 open elements. A deeper start tag is dropped, and its text joins the deepest element | Browsers cap depth, and the scope rules cost time in proportion to it. |
| `<template>` | Content skipped by element | The scan skips everything from `<template>` to its end tag, nesting included, and never gives it to the tree builder | The content is never prose, and a template leaves markers in the tree builder's list of formatting elements that cost time without showing in any stack depth. |
| `<object>`, `<applet>`, `<marquee>` | Plain tags | The tags are dropped and the content, including the fallback of an `<object>`, reads as part of the surrounding text | These elements leave the same markers. |
| Nested tables | No limit | At most 16 open tables. A `<table>` start tag inside 16 is dropped | Cells and captions leave markers, so the cap bounds the list of formatting elements. |
| Input size | No limit | A body over 16 MiB is refused with `ParseError::TooLarge`. The caller reports the check as incomplete | Any cost that stays polynomial is bounded in absolute terms. |
| Work limit | None | A document that needs more than 32 units of work a byte (plus 4,096) is refused with `ParseError::Budget`. The caller reports the check as incomplete | Hostile markup can make a tree builder do far more work than its size. Prose costs about 2 units a byte. |
| Text a tree builder moves | Not applicable | Text that a table pulls in front of it is a block of its own, and blocks keep source order | Projections must ascend in the source. |

## Differential test

`tests/html_differential.rs` compares the prose with the text that html5ever's own parser renders, over a corpus and generated tag soup, with LF, CRLF and CR line endings. The extractor uses the same tree builder, so the test mostly checks the walk, the provenance of text, and the hand-off of tokens and tokenizer states. A document with a table may differ in word order, because a table pulls some text in front of itself and the extractor keeps source order. Those documents must still hold the same words.

`cargo test --release -p slopvac-parse --test html_differential -- --ignored` runs 400,000 more documents.
