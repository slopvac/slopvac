//! Vale parity: the crate's tags against the tags Vale 3.27 (prose v3.3.1)
//! reports for the same text. The expected values are recorded in
//! `tests/fixtures/`; `tools/gen_fixtures.py --check` proves they are current.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use slopvac_pos::Tagger;

const GOLD: &str = include_str!("fixtures/gold.tsv");
const CORPUS_MD: &str = include_str!("fixtures/corpus.md");
const CORPUS_TSV: &str = include_str!("fixtures/corpus.tsv");
const SHIPPED_BLOCKLIST: &str =
    include_str!("../../../packages/slopvac-lint/examples/blocklist.toml");

/// Floors the acceptance criteria set (99% exact match, 95% blocklist
/// agreement), and the numbers measured when this test was written. The
/// measured exact-match count must not regress; raise it when the port
/// improves. The 4 differences are mostly Vale's own reporting artefacts next
/// to code spans (its star placeholder, and the word after a span, are
/// reported at the wrong column); the rest are single-word tagger differences.
const CORPUS_EXACT_FLOOR: f64 = 0.99;
const CORPUS_TOKENS: usize = 17_173;
const CORPUS_EXACT_MEASURED: usize = 17_169;
const BLOCKLIST_AGREEMENT_FLOOR: f64 = 0.95;

/// `(line, byte offset in the line)` of a token, both Vale's and ours.
type Pos = (usize, usize);

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

fn position(starts: &[usize], offset: usize) -> Pos {
    let line = starts.partition_point(|&s| s <= offset);
    (line, offset - starts[line - 1])
}

/// Our tags for the corpus slice, by position.
fn our_corpus_tags() -> BTreeMap<Pos, (String, String)> {
    let starts = line_starts(CORPUS_MD);
    Tagger::english()
        .tag_text(CORPUS_MD)
        .into_iter()
        .map(|t| {
            (
                position(&starts, t.span.start),
                (t.text.to_owned(), t.tag.to_owned()),
            )
        })
        .collect()
}

/// Vale's tags for the corpus slice: position, word and tag.
fn vale_corpus_tags() -> Vec<(Pos, String, String)> {
    CORPUS_TSV
        .lines()
        .map(|row| {
            let f: Vec<&str> = row.split('\t').collect();
            (
                (f[0].parse().unwrap(), f[1].parse().unwrap()),
                f[2].to_owned(),
                f[3].to_owned(),
            )
        })
        .collect()
}

#[test]
fn all_143_gold_targets_get_the_tag_vale_gives() {
    let mut checked = 0;
    let mut wrong = Vec::new();
    for row in GOLD.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        let (sentence, word, offset, vale) = (f[0], f[1], f[2].parse::<usize>().unwrap(), f[4]);
        let tokens = Tagger::english().tag_text(sentence);
        let got = tokens
            .iter()
            .find(|t| t.span.start == offset && t.text == word)
            .map(|t| t.tag);
        if got != Some(vale) {
            wrong.push(format!(
                "{word:?} in {sentence:?}: got {got:?}, Vale {vale}"
            ));
        }
        checked += 1;
    }
    assert_eq!(checked, 143);
    assert!(
        wrong.is_empty(),
        "{} of 143 differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn corpus_tags_match_vale_at_least_99_percent() {
    let ours = our_corpus_tags();
    let vale = vale_corpus_tags();
    let exact = vale
        .iter()
        .filter(|(pos, word, tag)| ours.get(pos).is_some_and(|(w, t)| w == word && t == tag))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let rate = exact as f64 / vale.len() as f64;
    eprintln!("corpus exact match: {exact}/{} = {rate:.4}", vale.len());
    assert_eq!(vale.len(), CORPUS_TOKENS, "fixture size changed");
    assert!(
        rate >= CORPUS_EXACT_FLOOR,
        "exact match {rate:.4} is below 99%"
    );
    assert!(
        exact >= CORPUS_EXACT_MEASURED,
        "exact matches fell to {exact}, was {CORPUS_EXACT_MEASURED}"
    );
}

#[derive(Deserialize)]
struct Blocklist {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    word: String,
    pos: String,
}

/// The Penn tag test of each blocklist part of speech, as slopvac's Vale
/// compiler writes it (`PENN_TAGS` in `compile_vale.py`). Vale matches the tag
/// regex unanchored, so `RB` also matches `WRB`.
fn pos_matches(pos: &str, tag: &str) -> bool {
    let family = match pos {
        "noun" => "NN",
        "verb" => "VB",
        "adjective" => "JJ",
        "adverb" => "RB",
        other => panic!("unexpected part of speech {other}"),
    };
    tag.contains(family)
}

const POSES: [&str; 4] = ["noun", "verb", "adjective", "adverb"];

/// Finding agreement `|both| / |union|` between Vale and this crate for one
/// blocklist, per part of speech: `(both, union)`.
fn agreement(list: &BTreeMap<&str, BTreeSet<String>>) -> BTreeMap<&'static str, (usize, usize)> {
    let ours = our_corpus_tags();
    let vale = vale_corpus_tags();
    POSES
        .iter()
        .map(|&pos| {
            let words = &list[pos];
            let hit = |word: &str, tag: &str| {
                words.contains(&word.to_lowercase()) && pos_matches(pos, tag)
            };
            let theirs: BTreeSet<Pos> = vale
                .iter()
                .filter(|(_, w, t)| hit(w, t))
                .map(|(p, _, _)| *p)
                .collect();
            let mine: BTreeSet<Pos> = ours
                .iter()
                .filter(|(_, (w, t))| hit(w, t))
                .map(|(p, _)| *p)
                .collect();
            (
                pos,
                (
                    theirs.intersection(&mine).count(),
                    theirs.union(&mine).count(),
                ),
            )
        })
        .collect()
}

fn assert_agreement(label: &str, list: &BTreeMap<&str, BTreeSet<String>>) {
    for (pos, (both, union)) in agreement(list) {
        assert!(
            union > 0,
            "{label} {pos}: the slice has no candidate findings"
        );
        #[allow(clippy::cast_precision_loss)]
        let rate = both as f64 / union as f64;
        eprintln!("{label} {pos}: {both}/{union} = {rate:.4}");
        assert!(
            rate >= BLOCKLIST_AGREEMENT_FLOOR,
            "{label} {pos}: agreement {rate:.4} is below 95% ({both}/{union})"
        );
    }
}

#[test]
fn shipped_blocklist_findings_agree_with_vale_per_pos() {
    let parsed: Blocklist = toml::from_str(SHIPPED_BLOCKLIST).unwrap();
    let mut list: BTreeMap<&str, BTreeSet<String>> =
        POSES.iter().map(|&p| (p, BTreeSet::new())).collect();
    for e in parsed.entries {
        // Mirrors `vocabulary_sequence_rules`: alphabetic, three letters or more.
        if e.word.chars().all(char::is_alphabetic) && e.word.chars().count() >= 3 {
            list.get_mut(e.pos.as_str())
                .unwrap()
                .insert(e.word.to_lowercase());
        }
    }
    assert_agreement("shipped", &list);
}

#[test]
fn stress_blocklist_findings_agree_with_vale_per_pos() {
    // Every ambiguous word of the gold set, blocked as every part of speech.
    let words: BTreeSet<String> = GOLD
        .lines()
        .map(|row| row.split('\t').nth(1).unwrap().to_lowercase())
        .collect();
    let list: BTreeMap<&str, BTreeSet<String>> =
        POSES.iter().map(|&p| (p, words.clone())).collect();
    assert_agreement("stress", &list);
}
