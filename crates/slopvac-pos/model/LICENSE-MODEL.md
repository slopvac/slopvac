# Model licence and attribution

The file `en.bin` in this directory is licensed under
[Creative Commons Attribution-ShareAlike 4.0 International][cc-by-sa-4.0]
(CC BY-SA 4.0). This notice covers `en.bin` only. The Rust code of the
`slopvac-pos` crate and the rest of the repository stay under Apache-2.0.

## What the file is

`en.bin` is the English part-of-speech model of [prose][prose] v3.3.1, an
averaged-perceptron tagger that uses Penn Treebank tags. Its own metadata
reads:

| Key | Value |
| --- | --- |
| `source` | `UD_English-EWT@4a4d77f (2026-05-06)` |
| `license` | `CC BY-SA 4.0` |
| `tagset` | `penn` |

SHA-256: `031ef22b05a03ea31ebc154bffd396e8497360546671d48f7b921f20ef4c9345`.
Size: 1,877,993 bytes.

## Source and authors

- Model: prose, by Joseph Kato and contributors. The file is
  `tag/aptagmodel/en.bin` in the `v3.3.1` tag of
  <https://github.com/jdkato/prose>.
- Training data: the [UD_English-EWT][ewt] treebank of the Universal
  Dependencies project (English Web Treebank; Silveira et al., 2014, "A Gold
  Standard Dependency Corpus for English"), released under CC BY-SA 4.0.
  Credit goes to the treebank's annotators and to the Universal Dependencies
  contributors.

## Changes

None. `en.bin` is byte for byte the file from the prose `v3.3.1` tag.

## Share-alike

If you adapt `en.bin`, for example by retraining or converting it, you must
license your adapted model under CC BY-SA 4.0 or a compatible licence, keep this
attribution, and say what you changed. A program that only reads the model
does not become a derivative of it. Keep this notice with every copy of
`en.bin`.

[cc-by-sa-4.0]: https://creativecommons.org/licenses/by-sa/4.0/
[prose]: https://github.com/jdkato/prose
[ewt]: https://github.com/UniversalDependencies/UD_English-EWT
