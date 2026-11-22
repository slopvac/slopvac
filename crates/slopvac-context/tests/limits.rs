//! A pattern that hits the PCRE2 match limit only surfaces as an error when the tagger
//! reaches that pattern for a block it could tag.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use slopvac_context::{PlainDocument, Tagger, TaggerSpec};

/// Splits a run of `a` in about 2^40 ways before it fails on the final `b`. The same pattern
/// and subject stop at the PCRE2 match limit in `slopvac-pcre2`'s limit tests.
const CATASTROPHIC: &str = r"^(a|aa)+$";

fn catastrophic_heading() -> String {
    format!("# {}b\n", "a".repeat(64))
}

fn tagger(when: &str) -> Box<dyn Tagger> {
    let yaml = format!("schema_version: 3\nid: t\ntag: k\nscope: paragraph\nwhen:\n{when}");
    Box::new(TaggerSpec::from_yaml(&yaml).unwrap().compile().unwrap())
}

#[test]
fn a_heading_pattern_is_not_run_for_blocks_the_block_kind_excludes() {
    let doc = PlainDocument::from_markdown("a.md", &catastrophic_heading());
    let t = tagger(&format!(
        "  block_kind: paragraph\n  heading_path: '{CATASTROPHIC}'\n"
    ));
    assert_eq!(t.tag(&doc).unwrap().len(), 0);
}

#[test]
fn a_heading_pattern_that_hits_the_match_limit_is_an_error_for_a_block_it_could_tag() {
    let doc = PlainDocument::from_markdown("a.md", &format!("{}\nText.\n", catastrophic_heading()));
    let t = tagger(&format!(
        "  block_kind: paragraph\n  heading_path: '{CATASTROPHIC}'\n"
    ));
    assert!(t.tag(&doc).is_err());
}

#[test]
fn a_matching_ancestor_heading_spares_a_descendant_heading_that_hits_the_match_limit() {
    // `aaaa` matches the pattern. The 64 `a`s and a `b` beneath it would hit the match limit,
    // but the walk stops at the first matching heading and never reaches that one.
    let src = format!("# aaaa\n\n## {}b\n\nText.\n", "a".repeat(64));
    let doc = PlainDocument::from_markdown("a.md", &src);
    let t = tagger(&format!(
        "  block_kind: paragraph\n  heading_path: '{CATASTROPHIC}'\n"
    ));
    let tags = t.tag(&doc).unwrap();
    let got: Vec<(&str, &str)> = tags
        .iter()
        .map(|tag| (&src[tag.span.clone()], &*tag.evidence))
        .collect();
    assert_eq!(got, [("Text.", "t: block")]);
}
