# Divergences from the Python reference

`slopvac-parse` reads Markdown and MDX with real parsers. The Python projection
(`slopvac.projection.project`) is a line scanner. `reference.json` holds documents where the
two agree on every word and on the source text of every word. The golden suite
(slopvac-8l4.2.7) should expect the differences below.

| Area | Python | Rust | Why |
| --- | --- | --- | --- |
| Entity token range | Empty source range, so an entity at the end of a word is left out of the word's span | The whole entity | The position contract fixes this in 3.0. See "What changes in 3.0" in `docs/contract/positions.md`. |
| Backslash escapes | Not decoded; `\*` stays two characters | One `backslash` token that covers both bytes | Markdown semantics. |
| Code span with a backtick inside | Not recognized; ``a`b`` leaks into prose | Masked as one span | Python's span pattern forbids a backtick in the body. |
| Single `*x*` and `_x_` emphasis | Marker characters stay in the prose | Markers are dropped | Python only strips `**`, `__` and `~~`. |
| Tables | The row text with its pipes, and the delimiter row as prose | Each cell is a block; the delimiter row is not prose | The brief asks for cells as blocks. |
| Images | The alt text is prose | The alt text is not prose | Alt text is not part of the reading flow. |
| Autolinks `<https://x>` | Dropped like a tag | The URL is prose | CommonMark makes them links. |
| HTML blocks | Tags stripped per line, text kept | Not prose | HTML text extraction is slopvac-8l4.3.5. |
| Setext headings and `---` rules | Read as paragraph text | Headings and rules | Markdown semantics. |
| Admonitions | `:::` and `!!!` lines are prose, and a `!!!` body is skipped as indented code | Markers are masked; the body is prose | New in 3.0. |
| Heading attributes | `{#id}` stays in the heading | Stripped | New in 3.0. |
| Masked code placeholder | A NUL with a space on each side | A single NUL | The position vectors use one NUL. The fixture compares words with NUL read as a space. |
| Block joins | One synthetic space between any two blocks | Blocks are separate; the engine decides | Python projects a whole document at once. |
| Inline comments | Dropped | A masked token with empty text | The position vectors say so. |
| Front matter | Detected inside `project` | Not detected; the caller passes the body range | Front matter is slopvac-8l4.3.4. |

## MDX

| Area | Behavior |
| --- | --- |
| Expressions | markdown-rs gets a brace checker that skips string, template and comment text, because the library has no JavaScript parser. An expression, inline or standalone, is prose only when it is one plain string literal; it becomes an `opaque` token. Other expressions render nothing. An unfinished expression with more than 1000 closing braces is a `ParseError::Mdx`, because markdown-rs rebuilds the expression text on every closing brace. |
| ESM | `import` and `export` blocks are accepted and are not prose. The code is not validated. |
| Fence closers | MDX has no indented code, and markdown-rs lets a closing fence use any indent. Markdown allows three columns past the opener. The admonition scan follows each parser. |
| Table cells | markdown-rs includes the leading pipe in a cell's span. Cell prose spans are exact. |
| `<` and comments | `<` starts a JSX tag and `<!-- -->` is a syntax error, so MDX documents fail with `ParseError::Mdx`. |

## Limits of the admonition scan

The scan runs per line before parsing. Markers must start a line, with up to three spaces
of indent after any `!!!` body indent. Markers inside block quotes or list items are not
recognized. A code body inside a block quote keeps the `>` prefixes of its lines, because a body is one contiguous byte range; the closing fence is not part of it. A GFM alert continues over following `>` lines only. A `:::` line that has a
word after the colons is an opener, so a prose line such as `::: and so on` is read as one.
