//! The averaged-perceptron model: reader for prose's flat model format, and the
//! scoring that follows prose v3.3.1's `tag` and `internal/perceptron` packages.

use std::collections::HashMap;

use crate::go_unicode;
use crate::tokenize::work;

/// Why a model file was rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// The file does not start with the `PROSETAG` magic.
    #[error("not a prose tagger model (bad magic)")]
    BadMagic,
    /// The file ends before a field it declares.
    #[error("model file is truncated")]
    Truncated,
    /// A field holds a value the format does not allow.
    #[error("model file is corrupt: {0}")]
    Corrupt(&'static str),
    /// The model uses a feature template this port does not implement.
    #[error("unsupported feature template {0:?}; only the rune template (1) is implemented")]
    UnsupportedTemplate(String),
    /// The model emits a tag set other than Penn Treebank.
    #[error("unsupported tag set {0:?}; only \"penn\" is implemented")]
    UnsupportedTagset(String),
}

/// A trained tagger model.
#[derive(Debug)]
pub(crate) struct Model {
    classes: Vec<Box<str>>,
    /// Feature key to its `[lo, hi)` range in `class_ids` and `weights`.
    features: HashMap<Box<str>, (u32, u32)>,
    class_ids: Vec<u8>,
    weights: Vec<f32>,
    /// Words the training data only ever saw with one tag.
    unambiguous: HashMap<Box<str>, u8>,
    meta: HashMap<Box<str>, Box<str>>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ModelError> {
        let end = self.pos.checked_add(n).ok_or(ModelError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(ModelError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn u32(&mut self) -> Result<u32, ModelError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn len(&mut self) -> Result<usize, ModelError> {
        usize::try_from(self.u32()?).map_err(|_| ModelError::Corrupt("length overflows usize"))
    }

    fn u8(&mut self) -> Result<u8, ModelError> {
        Ok(self.take(1)?[0])
    }

    fn str16(&mut self) -> Result<Box<str>, ModelError> {
        let b = self.take(2)?;
        let n = usize::from(u16::from_le_bytes([b[0], b[1]]));
        let raw = self.take(n)?;
        std::str::from_utf8(raw)
            .map(Box::from)
            .map_err(|_| ModelError::Corrupt("string is not UTF-8"))
    }

    fn offsets(&mut self, n: usize) -> Result<Vec<u32>, ModelError> {
        let raw = self.take(n.checked_mul(4).ok_or(ModelError::Truncated)?)?;
        Ok(raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect())
    }
}

/// Checks that `offsets` is non-decreasing, starts at 0 and ends at `end`.
fn check_offsets(offsets: &[u32], end: usize) -> Result<(), ModelError> {
    let ok = offsets.first() == Some(&0)
        && offsets.windows(2).all(|w| w[0] <= w[1])
        && offsets.last().map(|&l| l as usize) == Some(end);
    if ok {
        Ok(())
    } else {
        Err(ModelError::Corrupt("offset table is not monotonic"))
    }
}

impl Model {
    pub(crate) fn parse(data: &[u8]) -> Result<Self, ModelError> {
        let mut r = Reader {
            bytes: data,
            pos: 0,
        };
        if r.take(8)? != b"PROSETAG" {
            return Err(ModelError::BadMagic);
        }
        let version = r.u32()?;
        let class_count = r.len()?;
        if class_count == 0 || class_count > 256 {
            return Err(ModelError::Corrupt("class count out of range"));
        }
        let classes = (0..class_count)
            .map(|_| r.str16())
            .collect::<Result<Vec<_>, _>>()?;
        let feature_count = r.len()?;
        let pair_count = r.len()?;
        let blob_len = r.len()?;
        let blob = r.take(blob_len)?;
        let feat_off = r.offsets(feature_count + 1)?;
        let pair_off = r.offsets(feature_count + 1)?;
        check_offsets(&feat_off, blob_len)?;
        check_offsets(&pair_off, pair_count)?;
        let class_ids = r.take(pair_count)?.to_vec();
        if class_ids.iter().any(|&c| usize::from(c) >= class_count) {
            return Err(ModelError::Corrupt("class id out of range"));
        }
        let weight_bytes = r.take(pair_count.checked_mul(4).ok_or(ModelError::Truncated)?)?;
        let weights = weight_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();

        let word_count = r.len()?;
        // Each record takes at least three bytes (length, one tag byte), so a
        // count beyond that is a truncated file; check before reserving.
        if word_count > r.remaining() / 3 {
            return Err(ModelError::Truncated);
        }
        let mut unambiguous = HashMap::with_capacity(word_count);
        for _ in 0..word_count {
            let word = r.str16()?;
            let id = r.u8()?;
            if usize::from(id) >= class_count {
                return Err(ModelError::Corrupt("tag id out of range"));
            }
            unambiguous.insert(word, id);
        }

        let mut meta = HashMap::new();
        if version >= 2 {
            let count = r.len()?;
            // Each entry takes at least four bytes (two empty strings).
            if count > r.remaining() / 4 {
                return Err(ModelError::Truncated);
            }
            for _ in 0..count {
                let key = r.str16()?;
                let value = r.str16()?;
                meta.insert(key, value);
            }
        }

        let mut features = HashMap::with_capacity(feature_count);
        for j in 0..feature_count {
            let key = std::str::from_utf8(&blob[feat_off[j] as usize..feat_off[j + 1] as usize])
                .map_err(|_| ModelError::Corrupt("feature key is not UTF-8"))?;
            features.insert(Box::from(key), (pair_off[j], pair_off[j + 1]));
        }

        let model = Self {
            classes,
            features,
            class_ids,
            weights,
            unambiguous,
            meta,
        };
        // Only the rune template (1) is implemented; the original byte
        // template (0, or no metadata) is not.
        match model.meta("template") {
            "1" => {}
            other => return Err(ModelError::UnsupportedTemplate(other.to_owned())),
        }
        match model.meta("tagset") {
            "penn" => {}
            other => return Err(ModelError::UnsupportedTagset(other.to_owned())),
        }
        Ok(model)
    }

    pub(crate) fn meta(&self, key: &str) -> &str {
        self.meta.get(key).map_or("", |v| &**v)
    }

    /// Tags one sentence; the result has one tag per word.
    pub(crate) fn tag<'a>(&'a self, words: &[&str]) -> Vec<&'a str> {
        work::tick(words.len() + 4);
        let mut ctx: Vec<String> = Vec::with_capacity(words.len() + 4);
        ctx.push("-START-".to_owned());
        ctx.push("-START2-".to_owned());
        ctx.extend(words.iter().map(|w| normalize(w)));
        ctx.push("-END-".to_owned());
        ctx.push("-END2-".to_owned());

        work::tick(self.classes.len());
        let mut scratch = Scratch {
            scores: vec![0.0; self.classes.len()],
            key: String::new(),
        };
        let mut p1: &'a str = "-START-";
        let mut p2: &'a str = "-START2-";
        let mut out: Vec<&'a str> = Vec::with_capacity(words.len());
        for (i, &word) in words.iter().enumerate() {
            work::tick(1);
            let tag = if let Some(tag) = fixed(word) {
                tag
            } else if let Some(&id) = self.unambiguous.get(word) {
                &*self.classes[usize::from(id)]
            } else {
                self.predict(&mut scratch, i + 2, &ctx, word, p1, p2)
            };
            p2 = p1;
            p1 = tag;
            out.push(tag);
        }
        out
    }

    /// Adds the weights of the feature `parts` concatenate to, if the model
    /// knows it. `key` is scratch space, reused across features.
    fn accumulate(&self, key: &mut String, parts: &[&str], scores: &mut [f32]) {
        key.clear();
        for part in parts {
            key.push_str(part);
        }
        work::tick(1 + key.len());
        if let Some(&(lo, hi)) = self.features.get(key.as_str()) {
            work::tick((hi - lo) as usize);
            for k in lo as usize..hi as usize {
                scores[usize::from(self.class_ids[k])] += self.weights[k];
            }
        }
    }

    /// Scores the rune-template features of `word` (at `ctx[i]`) and returns
    /// the best class. Ties go to the lowest class id, as in prose.
    /// `scratch` holds the score vector and key buffer, reused per sentence.
    fn predict(
        &self,
        scratch: &mut Scratch,
        i: usize,
        ctx: &[String],
        word: &str,
        p1: &str,
        p2: &str,
    ) -> &str {
        let Scratch { scores, key } = scratch;
        work::tick(scores.len());
        scores.fill(0.0);
        let suffix = suffix_chars(word, 3);
        // One entry per feature, in the order prose builds them; each is
        // the feature letter followed by its parts.
        let features: [&[&str]; 20] = [
            &["a"],
            &["b", suffix],
            &["c", prefix_chars(word, 1)],
            &["d", p1],
            &["e", p2],
            &["f", p1, " ", p2],
            &["g", &ctx[i]],
            &["h", p1, " ", &ctx[i]],
            &["i", &ctx[i - 1]],
            &["j", suffix_chars(&ctx[i - 1], 3)],
            &["k", &ctx[i - 2]],
            &["l", &ctx[i + 1]],
            &["m", suffix_chars(&ctx[i + 1], 3)],
            &["n", &ctx[i + 2]],
            &["o", suffix_chars(word, 2)],
            &["p", suffix_chars(word, 4)],
            &["q", prefix_chars(word, 2)],
            &["r", prefix_chars(word, 3)],
            &["s", shape(word)],
            &["t", p1, " ", suffix],
        ];
        for parts in features {
            self.accumulate(key, parts, scores);
        }

        work::tick(scores.len());
        let mut best = f32::NEG_INFINITY;
        let mut best_id = 0;
        for (id, &score) in scores.iter().enumerate() {
            if score > best {
                best = score;
                best_id = id;
            }
        }
        &self.classes[best_id]
    }
}

/// Per-sentence working memory for [`Model::predict`].
struct Scratch {
    scores: Vec<f32>,
    key: String,
}

/// The last `n` characters of `word`, or all of it if shorter.
fn suffix_chars(word: &str, n: usize) -> &str {
    let start = word
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    &word[start..]
}

/// The first `n` characters of `word`, or all of it if shorter.
fn prefix_chars(word: &str, n: usize) -> &str {
    let end = word.char_indices().nth(n).map_or(word.len(), |(i, _)| i);
    &word[..end]
}

/// Casing and makeup of a word in a few letters: `Xx` capitalized, `XX` all
/// capitals, `xx` lower case, `xX` mixed, `d` digits, `p` punctuation, `o`
/// anything else. Character classes are Go's (`unicode.IsUpper` and so on),
/// not Rust's.
fn shape(word: &str) -> &'static str {
    work::tick(word.len());
    let (mut upper, mut lower, mut digit, mut punct) = (0_usize, 0_usize, 0_usize, 0_usize);
    for c in word.chars() {
        if go_unicode::is_upper(c) {
            upper += 1;
        } else if go_unicode::is_lower(c) {
            lower += 1;
        } else if go_unicode::is_digit(c) {
            digit += 1;
        } else if go_unicode::is_punct_or_symbol(c) {
            punct += 1;
        }
    }
    let letters = upper + lower;
    if letters == 0 {
        return if digit > 0 {
            "d"
        } else if punct > 0 {
            "p"
        } else {
            "o"
        };
    }
    if lower == 0 {
        "XX"
    } else if upper == 0 {
        "xx"
    } else if upper == 1 && word.chars().next().is_some_and(go_unicode::is_upper) {
        "Xx"
    } else {
        "xX"
    }
}

/// Maps a word onto the form the model was trained against.
fn normalize(word: &str) -> String {
    work::tick(1 + word.len());
    let Some(&first) = word.as_bytes().first() else {
        return String::new();
    };
    if first != b'-' && word.contains('-') {
        return "!HYPHEN".to_owned();
    }
    if word.len() == 4 && is_decimal_int(word) {
        return "!YEAR".to_owned();
    }
    if first.is_ascii_digit() {
        return "!DIGITS".to_owned();
    }
    // Go lower-cases rune by rune with the simple mapping (`unicode.ToLower`):
    // U+0130 becomes `i`, and a capital sigma is never the final sigma. Rust's
    // `str::to_lowercase` would expand the first and contextualise the second.
    word.chars().map(go_unicode::to_lower).collect()
}

/// Whether `s` is an optional sign followed by at least one ASCII digit (what
/// Go's `strconv.Atoi` accepts, for the four-byte inputs it is used on).
fn is_decimal_int(s: &str) -> bool {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Tags the model never decides: `-` itself, mentions (`@name`), Penn Treebank
/// traces (`*T*-1`, `0`, `*`) and bracket literals (`-LRB-`).
fn fixed(word: &str) -> Option<&'static str> {
    if word == "-" {
        Some("-")
    } else if word.starts_with('@') {
        Some("NN")
    } else if is_trace(word) {
        Some("-NONE-")
    } else if let "-LRB-" | "-RRB-" = word {
        // Bracket literals keep their own tag. prose accepts any `-XXX-`; the
        // model only has these two tags, so any other word is left to it.
        Some(if word == "-LRB-" { "-LRB-" } else { "-RRB-" })
    } else {
        None
    }
}

/// Go: `^(?:0|\*[\w?]\*|\*\-\d{1,3}|\*[A-Z]+\*\-\d{1,3}|\*)$`.
fn is_trace(word: &str) -> bool {
    let digits = |s: &str| (1..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    let b = word.as_bytes();
    if word == "0" || word == "*" {
        return true;
    }
    if b.len() == 3 && b[0] == b'*' && b[2] == b'*' {
        return b[1].is_ascii_alphanumeric() || b[1] == b'_' || b[1] == b'?';
    }
    if let Some(rest) = word.strip_prefix("*-") {
        return digits(rest);
    }
    if let Some(rest) = word.strip_prefix('*')
        && let Some((name, num)) = rest.split_once("*-")
    {
        return !name.is_empty() && name.bytes().all(|c| c.is_ascii_uppercase()) && digits(num);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(
            Model::parse(b"NOTAMODEL...").unwrap_err(),
            ModelError::BadMagic
        );
    }

    #[test]
    fn rejects_truncated_input() {
        assert_eq!(
            Model::parse(b"PROSETAG\x02").unwrap_err(),
            ModelError::Truncated
        );
    }

    #[test]
    fn rejects_truncated_embedded_model() {
        let full = include_bytes!("../model/en.bin");
        assert!(Model::parse(&full[..full.len() / 2]).is_err());
    }

    #[test]
    fn embedded_model_declares_penn_english() {
        let model = Model::parse(include_bytes!("../model/en.bin")).unwrap();
        assert_eq!(model.meta("tagset"), "penn");
        assert_eq!(model.meta("license"), "CC BY-SA 4.0");
    }

    #[test]
    fn traces_and_literals_are_fixed_without_the_model() {
        for (word, tag) in [
            ("-", Some("-")),
            ("@alice", Some("NN")),
            ("0", Some("-NONE-")),
            ("*", Some("-NONE-")),
            ("*T*", Some("-NONE-")),
            ("*-12", Some("-NONE-")),
            ("*-1234", None),
            ("*NP*-3", Some("-NONE-")),
            ("*np*-3", None),
            ("-LRB-", Some("-LRB-")),
            ("-lrb-", None),
            ("00", None),
        ] {
            assert_eq!(fixed(word), tag, "{word}");
        }
    }

    #[test]
    fn normalize_follows_prose() {
        for (word, want) in [
            ("Deploy", "deploy"),
            ("2024", "!YEAR"),
            ("-2024", "-2024"),
            ("12", "!DIGITS"),
            ("non-interactive", "!HYPHEN"),
            ("-", "-"),
            ("", ""),
        ] {
            assert_eq!(normalize(word), want, "{word}");
        }
    }

    /// Go's `unicode.ToLower` per rune: dotted capital I is `i` (Rust's
    /// whole-string lowering gives `i` plus a combining dot), and a capital
    /// sigma at the end of a word stays `\u{3c3}` (Rust gives final sigma).
    #[test]
    fn normalize_lowers_rune_by_rune_like_go() {
        assert_eq!(normalize("\u{130}stanbul"), "istanbul");
        assert_eq!(
            normalize("\u{39f}\u{394}\u{39f}\u{3a3}"),
            "\u{3bf}\u{3b4}\u{3bf}\u{3c3}"
        );
    }

    /// Go's `unicode.IsDigit` is Nd only: Arabic-Indic digits count,
    /// superscripts and roman numerals do not.
    #[test]
    fn shape_counts_decimal_digits_only() {
        for (word, want) in [
            ("12", "d"),
            ("\u{663}\u{664}", "d"),
            ("\u{ff11}\u{ff12}", "d"),
            ("\u{b2}", "o"),
            ("\u{bd}", "o"),
            ("\u{2167}", "o"),
            ("\u{24b6}", "p"),
            ("\u{1c5}", "o"),
            ("\u{1c5}a", "xx"),
            ("A\u{2167}", "XX"),
            ("Ab\u{2167}", "Xx"),
            ("\u{2167}A", "XX"),
        ] {
            assert_eq!(shape(word), want, "{word}");
        }
    }
}

/// Tagging with a defect put back, for the work-budget tests.
#[cfg(test)]
pub(crate) mod oracle {
    use super::*;

    /// `Model::tag`, with the context of every word built again from the
    /// whole sentence: quadratic in the sentence length.
    pub(crate) fn tag_rebuilding_context<'a>(model: &'a Model, words: &[&str]) -> Vec<&'a str> {
        let mut scratch = Scratch {
            scores: vec![0.0; model.classes.len()],
            key: String::new(),
        };
        let (mut p1, mut p2): (&'a str, &'a str) = ("-START-", "-START2-");
        let mut out = Vec::with_capacity(words.len());
        for (i, &word) in words.iter().enumerate() {
            work::tick(words.len() + 4);
            let mut ctx: Vec<String> = vec!["-START-".to_owned(), "-START2-".to_owned()];
            ctx.extend(words.iter().map(|w| normalize(w)));
            ctx.push("-END-".to_owned());
            ctx.push("-END2-".to_owned());
            let tag = model.predict(&mut scratch, i + 2, &ctx, word, p1, p2);
            p2 = p1;
            p1 = tag;
            out.push(tag);
        }
        out
    }
}
