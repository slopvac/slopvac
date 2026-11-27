//! The checks the terms and blocklist readers make, held to the patterns and lists that the
//! schemas state. The reader and the schema must give one verdict on any string, so a file an
//! editor accepts is a file `slopvac` loads. Each property builds a small file around one string
//! and compares the reader with the schema member that names the same rule.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::path::Path;
use std::sync::LazyLock;

use jsonschema::Validator;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_terms::{Blocklist, Pos, TermsFile};

/// The schema member at `pointer` of the published `file`, as a validator of one string.
fn member(file: &str, pointer: &str) -> Validator {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3")
        .join(file);
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let member = schema
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{file} has no {pointer}"));
    jsonschema::validator_for(member).unwrap()
}

static KIND: LazyLock<Validator> =
    LazyLock::new(|| member("terms.schema.json", "/$defs/Term/properties/kind"));
static WORD: LazyLock<Validator> = LazyLock::new(|| {
    member(
        "blocklist.schema.json",
        "/$defs/BlocklistEntry/properties/word",
    )
});
static REASON: LazyLock<Validator> = LazyLock::new(|| {
    member(
        "blocklist.schema.json",
        "/$defs/BlocklistEntry/properties/reason",
    )
});
static POS: LazyLock<Validator> = LazyLock::new(|| {
    member(
        "blocklist.schema.json",
        "/$defs/BlocklistEntry/properties/pos",
    )
});

fn quoted(s: &str) -> String {
    toml::Value::from(s).to_string()
}

fn kind_loads(kind: &str) -> bool {
    let text = format!(
        "schema_version = 3\n[[terms]]\nterm = \"x\"\nkind = {}\n",
        quoted(kind)
    );
    TermsFile::parse("t.toml", &text).is_ok()
}

fn entry_loads(word: &str, pos: &str, reason: &str) -> bool {
    let text = format!(
        "schema_version = 3\n[[entries]]\nword = {}\npos = {}\nreason = {}\n",
        quoted(word),
        quoted(pos),
        quoted(reason)
    );
    Blocklist::parse("b.toml", "toml", &text).is_ok()
}

fn schema_accepts(validator: &Validator, s: &str) -> bool {
    validator.is_valid(&json!(s))
}

/// Strings near the edges of the kebab-case label, and arbitrary text.
fn kind_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 \\-_\n]{0,6}",
        "(proper-noun|a|a-|-a|a1|1a|A)[a-z\n ]{0,2}",
        any::<String>(),
    ]
}

/// The two code points where the reader and the published patterns disagree about white space.
/// The explicit cases below record each side, and the properties leave them out.
const ENGINES_DIFFER: [char; 2] = ['\u{85}', '\u{feff}'];

/// Strings built from white space characters and letters, and arbitrary text, without the
/// code points of [`ENGINES_DIFFER`].
fn blank_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ \t\n\r\u{b}\u{c}\u{a0}\u{2028}\u{3000}]{0,4}[a-z]?",
        any::<String>(),
    ]
    .prop_filter("the engines differ on these", |s| {
        !s.contains(ENGINES_DIFFER)
    })
}

/// The published forms of a part of speech, mutated: case flips, padding and a trailing newline.
fn pos_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "(n|noun|NOUN|v|verb|VERB|adj|ADJ|adv|ADP|adp|CCONJ|SCONJ|conj|PRON|art|DET|det|Noun)[ \n]?",
        " ?[a-zA-Z]{0,5}",
        any::<String>(),
    ]
}

proptest! {
    #[test]
    fn the_kind_check_and_pattern_agree(s in kind_text()) {
        prop_assert_eq!(kind_loads(&s), schema_accepts(&KIND, &s));
    }

    #[test]
    fn the_word_check_and_pattern_agree(s in blank_text()) {
        prop_assert_eq!(entry_loads(&s, "noun", "r"), schema_accepts(&WORD, &s));
    }

    #[test]
    fn the_reason_check_and_pattern_agree(s in blank_text()) {
        prop_assert_eq!(entry_loads("w", "noun", &s), schema_accepts(&REASON, &s));
    }

    #[test]
    fn the_pos_check_and_list_agree(s in pos_text()) {
        prop_assert_eq!(Pos::parse(&s).is_some(), schema_accepts(&POS, &s));
        prop_assert_eq!(entry_loads("w", &s, "r"), schema_accepts(&POS, &s));
    }
}

/// The two code points where `\s` and `\S` read differently from `char::is_whitespace`.
/// The reader trims with `char::is_whitespace`, which counts U+0085 as white space and U+FEFF as
/// not. ECMA-262, and the validator that translates the published pattern to it, take the other
/// side of each: `\S` matches U+0085 and does not match U+FEFF. So a word or reason of only
/// U+FEFF loads but fails the schema, and one of only U+0085 is refused by the reader but passes
/// the schema. The published patterns stay as they are, and the reader does not change.
#[rstest]
#[case::byte_order_mark(0xFEFF, true, false)]
#[case::next_line(0x85, false, true)]
fn the_reader_and_the_schema_take_opposite_sides_where_engines_disagree(
    #[case] code: u32,
    #[case] reader_loads: bool,
    #[case] schema_accepts_it: bool,
) {
    let point = char::from_u32(code).unwrap().to_string();
    assert_eq!(
        entry_loads(&point, "noun", "r"),
        reader_loads,
        "word reader"
    );
    assert_eq!(
        schema_accepts(&WORD, &point),
        schema_accepts_it,
        "word schema"
    );
    assert_eq!(
        entry_loads("w", "noun", &point),
        reader_loads,
        "reason reader"
    );
    assert_eq!(
        schema_accepts(&REASON, &point),
        schema_accepts_it,
        "reason schema"
    );
}

#[rstest]
#[case::n("n")]
#[case::noun("noun")]
#[case::noun_tag("NOUN")]
#[case::v("v")]
#[case::verb("verb")]
#[case::verb_tag("VERB")]
#[case::adj("adj")]
#[case::adjective("adjective")]
#[case::adj_tag("ADJ")]
#[case::adv("adv")]
#[case::adverb("adverb")]
#[case::adv_tag("ADV")]
#[case::prep("prep")]
#[case::preposition("preposition")]
#[case::adp_tag("ADP")]
#[case::conj("conj")]
#[case::conjunction("conjunction")]
#[case::cconj_tag("CCONJ")]
#[case::sconj_tag("SCONJ")]
#[case::pron("pron")]
#[case::pronoun("pronoun")]
#[case::pron_tag("PRON")]
#[case::art("art")]
#[case::article("article")]
#[case::det_tag("DET")]
fn the_schema_lists_each_part_of_speech_the_reader_takes(#[case] form: &str) {
    assert!(schema_accepts(&POS, form));
}

#[rstest]
#[case::empty("")]
#[case::capitalised("Noun")]
#[case::padded(" noun")]
#[case::trailing_newline("noun\n")]
#[case::lowercase_tag("adp")]
#[case::unknown("interjection")]
fn a_part_of_speech_outside_the_list_is_refused_by_both(#[case] form: &str) {
    assert_eq!(Pos::parse(form), None);
    assert!(!schema_accepts(&POS, form));
}

#[test]
fn a_one_character_term_loads_and_passes_the_schema() {
    let text = "schema_version = 3\n[[terms]]\nterm = \"x\"\nvariants = [\"yy\"]\n";
    assert!(TermsFile::parse("t.toml", text).is_ok());
    let schema = member("terms.schema.json", "/$defs/Term");
    let instance = json!({"term": "x", "variants": ["yy"]});
    assert!(schema.is_valid(&instance));
}

#[test]
fn a_one_character_variant_loads_and_passes_the_schema() {
    let text = "schema_version = 3\n[[terms]]\nterm = \"xx\"\nvariants = [\"y\"]\n";
    assert!(TermsFile::parse("t.toml", text).is_ok());
    let schema = member("terms.schema.json", "/$defs/Term");
    let instance = json!({"term": "xx", "variants": ["y"]});
    assert!(schema.is_valid(&instance));
}
