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

`tag_text` takes UTF-8 prose. It skips inline code spans, keeps hyphenated
words whole, splits sentences, and reports a byte span in the input for each
token. It does not strip other Markdown or HTML markup; the caller passes in
prose.

## Accuracy

Against Vale 3.27 on a 20,130-word slice of the evaluation corpus, 99.97% of
tags are identical. All 143 hand-labelled gold targets get Vale's tag.
Blocklist findings agree with Vale at 100% per part of speech on the slice.
The tests in `tests/vale_parity.rs` assert these numbers.

`tools/gen_fixtures.py` regenerates the fixtures from Vale. Run it with
`--corpus DIR` to rewrite them, or add `--check` to compare. It needs Vale 3.27
or later.

## Licence

The code is Apache-2.0, like the rest of the repository. It follows prose's
tagger and tokenizer, which are MIT-licensed; credit goes to Joseph Kato and
the prose contributors.

The embedded model, `model/en.bin`, is licensed CC BY-SA 4.0, because it was
trained on UD_English-EWT. Its attribution and share-alike notice is in
[`model/LICENSE-MODEL.md`](model/LICENSE-MODEL.md); the notice ships with the
crate. The model is 1.9 MB and adds that much to the binary.

[prose]: https://github.com/jdkato/prose
