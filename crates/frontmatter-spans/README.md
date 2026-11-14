# frontmatter-spans

Detect YAML, TOML and JSON front matter at the start of a document and parse it. Every key and
value carries a source span, and each span indexes the whole document, not the front-matter block.

The crate does not depend on slopvac. It exists because no other crate gives front-matter values
spans: `gray_matter` has none, and `frontmatter-lossless` only finds the block.

## Use

```rust
use frontmatter_spans::FrontMatter;

let doc = "---\ntitle: Hello\n---\nBody\n";
let fm = FrontMatter::parse(doc).unwrap().unwrap();

let title = fm.get("title").unwrap();
assert_eq!(&doc[title.span.range.clone()], "Hello");
assert_eq!((title.span.start.line, title.span.start.column), (2, 8));
assert_eq!(&doc[fm.block.body_start..], "Body\n");
```

`FrontMatter::parse` returns `Ok(None)` when the document has no front matter and `Err` when a
block exists but does not parse. `detect` finds the block without parsing it.

| Format | Opens with | Closes with |
| --- | --- | --- |
| YAML | a `---` line | a `---` or `...` line |
| TOML | a `+++` line | a `+++` line |
| JSON | `{` | the matching `}` |

A leading UTF-8 byte-order mark is skipped. Lines end at LF, CRLF or a lone CR. A block that
never closes is not front matter. Trailing blanks after a delimiter are allowed.

## Spans

A `Span` holds a byte `range` and the 1-based `start` and `end` line and column. Columns count
code points, and the byte-order mark is not part of line 1. `LineIndex` gives the same mapping
for any offset in the document.

- A scalar span covers the value as written. Quoted scalars include the quotes.
- A YAML or JSON collection span runs from its first byte to its last, brackets included.
  Trailing newlines are not part of it.
- A TOML table span runs from its header, or its first key when it has none, to the end of its
  last entry.
- A key span covers the key as written, so JSON keys include their quotes.
- A YAML alias takes the anchored value and the span of the alias.

## Serde

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct Meta {
    genre: String,
}

let doc = "---\nslopvac:\n  genre: docs\n---\n";
let fm = frontmatter_spans::FrontMatter::parse(doc).unwrap().unwrap();
let meta = fm.deserialize_key::<Meta>("slopvac").unwrap().unwrap();
assert_eq!(meta.value.genre, "docs");
assert_eq!(&doc[meta.span.range], "genre: docs");
```

`FrontMatter::deserialize` reads the whole block. `deserialize_key` reads one top-level key and
returns the span of its value. When a value does not fit the type, `DeError::span` holds the
span of the value. For an unknown field under `deny_unknown_fields`, it holds the span of the
key.

## Parsers

- YAML uses `saphyr`, which reports character indexes. The crate converts them to byte offsets.
  Aliases expand within saphyr's node budget, so an alias bomb is an error.
- TOML uses `toml_edit`, which reports byte spans for keys and values.
- JSON uses a strict RFC 8259 parser inside the crate. The common JSON crates drop spans, and a
  YAML parser would accept comments and single quotes. Nesting deeper than 128 levels is an
  error.

YAML anchors, tags and non-scalar keys have limits. A tag is ignored. A non-scalar key is an
error. A block with more than one YAML document is an error.

## Tests and fuzzing

```sh
cargo test -p frontmatter-spans
cargo +nightly fuzz run parse   # from crates/frontmatter-spans, needs cargo-fuzz
```

The property tests check that spans lie inside the document on character boundaries, and that
written keys and integers sit at their spans in all three formats.

## Licence

Apache-2.0.
