# slopvac-pos

Part-of-speech tagging with Penn Treebank tags, for slopvac's vocabulary rules.

The crate ports the averaged-perceptron tagger of [prose][prose] v3.3.1, the
tagger Vale uses, and embeds prose's English model. Tags match Vale's, so
rules written against Vale's `NN`, `VB`, `JJ` and `RB` families keep their
meaning.

```rust
use slopvac_pos::Tagger;

let tagger = Tagger::english();
for token in tagger.tag_text("Test the endpoint with `curl`.") {
    println!("{} {} {:?}", token.text, token.tag, token.span);
}
```

`tag_text` takes UTF-8 prose. It replaces inline code spans with a placeholder
that conditions its neighbours' tags and is not reported, keeps hyphenated
words whole, splits sentences, and reports a byte span in the input for each
token. It does not strip other Markdown or HTML markup. A scope layer that
knows the blocks (paragraphs, list items, headings, table cells) passes their
byte ranges to `tag_blocks`, so sentences never cross a block, as in Vale.

## Accuracy against Vale 3.27

Vale tags with prose v3.3.1, so Vale's tags are the expected values.

In-tree regression tests (`tests/vale_parity.rs`, fixtures in `tests/fixtures/`):

| Check | Result |
| --- | --- |
| Gold examples with Vale's tag | 143 of 143 |
| Corpus slice exact tag match | 17,169 of 17,173 (99.98%) |
| Stress blocklist agreement | 100% per part of speech (noun 205 findings, verb 134, adjective 45, adverb 58) |
| Shipped blocklist agreement | 100% per part of speech, but on 1 to 6 findings each |

The slice has too few shipped-blocklist findings to show 95% per part of
speech. The full-corpus measurement does. It runs the 92-file evaluation
corpus of the 3.0 POS spike (slopvac-8l4.3.10) through Vale and through this
crate, and compares findings of the shipped blocklist
(`packages/slopvac-lint/examples/blocklist.toml`). The corpus and Vale are not
in the repository, so this is out-of-tree evidence:

```sh
cargo build -p slopvac-pos --example tag_dump
python3 crates/slopvac-pos/tools/full_corpus_agreement.py --corpus <corpus dir> --mask
```

Results of that run (Vale 3.27.0, 2026-11-14). The input is Markdown with its
markup blanked and one block per heading, list item, table row and paragraph,
a stand-in for the scope layer. Findings are keyed by file, line and column.

| Part of speech | Vale findings | Crate findings | Both | Agreement |
| --- | --- | --- | --- | --- |
| noun | 26 | 26 | 26 | 100% |
| verb | 38 | 38 | 38 | 100% |
| adjective | 28 | 28 | 28 | 100% |
| adverb | 50 | 50 | 50 | 100% |

On the 138,499 tokens that Vale and the crate both produce, 99.50% have the
same Penn tag. Without `--mask` the crate reads raw Markdown as one block, and
the agreement drops to 72% (noun), 86% (verb), 93% (adjective) and 100%
(adverb) over all findings. On tokens both sides share it stays at 95.5%,
97.4%, 100% and 100%, so the drop comes from scope (headings, links, HTML), not
the tagger.

`tools/gen_fixtures.py` regenerates the in-tree fixtures from Vale. Run it with
`--corpus DIR` to rewrite them, or add `--check` to compare. Both tools need
Vale 3.27 or later.

## Known differences from prose

- The emoticon shortcut (`:)` tagged `SYM`) is not ported.
- Only the rune-template Penn model that ships here is accepted.
- `-LRB-` and `-RRB-` are the only bracket literals pinned to their own tag.
- Word shape uses Rust's upper- and lower-case properties. Go uses the
  categories Lu and Ll, so a few letter-like symbols (roman numerals, circled
  letters) shape differently. Decimal digits and lower-casing follow Go.

## Licence

The code is Apache-2.0, like the rest of the repository. It is ported from
prose's tagger and tokenizer, which are MIT-licensed. [`LICENSE-prose`](LICENSE-prose)
holds prose's licence text with its copyright line and the upstream notices it
records; it ships with the crate.

The embedded model, `model/en.bin`, is licensed CC BY-SA 4.0, because it was
trained on UD_English-EWT. Its attribution and share-alike notice is in
[`model/LICENSE-MODEL.md`](model/LICENSE-MODEL.md); the notice ships with the
crate. The model is 1.9 MB and adds that much to the binary.

The test corpus slice quotes documentation from open-source projects. Each
source's files, copyright holder and licence text are in
[`tests/fixtures/NOTICES.md`](tests/fixtures/NOTICES.md).

[prose]: https://github.com/jdkato/prose
