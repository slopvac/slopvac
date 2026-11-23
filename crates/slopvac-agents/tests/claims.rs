//! Behaviour tests for `slopvac claims`: segmentation, alignment, literal diff.

use std::fs;

use slopvac_agents::claims::{self, Report, Summary, compare};

const ORIGINAL: &str = include_str!("fixtures/claims/original.md");
const REVISED: &str = include_str!("fixtures/claims/revised.md");

/// Facts the revision drops on purpose, as substrings of the original.
const PLANTED_DROPS: [&str; 3] = [
    "Entries expire after 24 hours.",
    "Each pack carries a minisign signature.",
    "The directory must be writable.",
];

fn kinds(list: &[claims::Literal]) -> Vec<(String, String)> {
    list.iter()
        .map(|l| (l.kind.to_owned(), l.text.clone()))
        .collect()
}

#[test]
fn fixture_summary_counts_match_the_hand_count() {
    let r = compare(ORIGINAL, REVISED).unwrap();
    assert_eq!(
        r.summary,
        Summary {
            original_sentences: 15,
            revised_sentences: 13,
            unchanged: 10,
            changed: 2,
            added: 1,
            dropped: 3,
        }
    );
}

#[test]
fn planted_drop_recall_and_precision_on_the_fixture() {
    let r = compare(ORIGINAL, REVISED).unwrap();
    let flagged: Vec<&str> = r
        .dropped
        .iter()
        .filter(|d| d.fact_candidate)
        .map(|d| d.text.as_str())
        .collect();
    let found = PLANTED_DROPS
        .iter()
        .filter(|p| flagged.contains(*p))
        .count();
    assert_eq!(found, PLANTED_DROPS.len(), "recall: flagged {flagged:?}");
    assert_eq!(
        flagged.len(),
        PLANTED_DROPS.len(),
        "precision: flagged {flagged:?}"
    );
}

#[test]
fn dropped_sentence_names_the_literal_the_revision_lost() {
    let r = compare(ORIGINAL, REVISED).unwrap();
    let d = r
        .dropped
        .iter()
        .find(|d| d.text.contains("24 hours"))
        .unwrap();
    assert_eq!(kinds(&d.lost_literals), [("number".into(), "24".into())]);
    assert_eq!(d.line, 3);
}

#[test]
fn changed_number_and_negation_are_reported_with_both_sentences() {
    let r = compare(ORIGINAL, REVISED).unwrap();
    let timeout = r
        .claims
        .iter()
        .find(|c| c.text.contains("timeout"))
        .unwrap();
    assert_eq!(timeout.change, "changed");
    assert_eq!(
        timeout.original.as_deref(),
        Some("The default timeout is 30 seconds.")
    );
    assert_eq!(
        kinds(&timeout.literals_removed),
        [("number".into(), "30".into())]
    );
    assert_eq!(
        kinds(&timeout.literals_added),
        [("number".into(), "60".into())]
    );
    let retry = r
        .claims
        .iter()
        .find(|c| c.text.contains("retries"))
        .unwrap();
    assert!(retry.negation_changed);
    assert!(!timeout.negation_changed);
}

#[test]
fn added_sentence_lists_its_literals() {
    let r = compare(ORIGINAL, REVISED).unwrap();
    let added = r.claims.iter().find(|c| c.change == "added").unwrap();
    assert_eq!(added.line, 11);
    assert_eq!(
        kinds(&added.literals_added),
        [("code".into(), "--refresh".into())]
    );
}

#[test]
fn a_merged_sentence_is_covered_not_a_dropped_fact() {
    let r = compare(
        "The tool reads the config. The tool reads the lockfile.",
        "The tool reads the config and the lockfile.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1);
    assert!(!r.dropped[0].fact_candidate, "{:?}", r.dropped[0]);
}

#[test]
fn identical_texts_report_nothing() {
    let r = compare(ORIGINAL, ORIGINAL).unwrap();
    assert!(r.claims.is_empty() && r.dropped.is_empty());
    assert_eq!(r.summary.unchanged, 15);
}

#[test]
fn literal_kinds_are_diffed() {
    let cases = [
        (
            "The limit is 10 requests.",
            "The limit is 20 requests.",
            "number",
            "10",
            "20",
        ),
        (
            "Pass --fast to run.",
            "Pass --slow to run.",
            "flag",
            "--fast",
            "--slow",
        ),
        (
            "Call load_config first.",
            "Call read_config first.",
            "identifier",
            "load_config",
            "read_config",
        ),
        (
            "Ask Alice for access.",
            "Ask Bob for access.",
            "name",
            "Alice",
            "Bob",
        ),
        (
            "Run `slopvac lint`.",
            "Run `slopvac check`.",
            "code",
            "slopvac lint",
            "slopvac check",
        ),
    ];
    for (old, new, kind, gone, came) in cases {
        let r = compare(old, new).unwrap();
        assert_eq!(r.claims.len(), 1, "{old}");
        let c = &r.claims[0];
        assert_eq!(
            kinds(&c.literals_removed),
            [(kind.into(), gone.into())],
            "{old}"
        );
        assert_eq!(
            kinds(&c.literals_added),
            [(kind.into(), came.into())],
            "{old}"
        );
    }
}

#[test]
fn abbreviations_decimals_and_code_spans_do_not_split_sentences() {
    let r = compare(
        "Use a tool, e.g. the linter. Version 3.0.0 ships soon. Run `a. B` now.",
        "",
    )
    .unwrap();
    assert_eq!(r.summary.original_sentences, 3);
}

#[test]
fn front_matter_is_skipped_and_fenced_code_is_one_unit() {
    let old = "---\ntitle: x\n---\nRun it.\n\n```sh\nslopvac lint a.md\n```\n";
    let new = "Run it.\n\n```sh\nslopvac lint b.md\n```\n";
    let r = compare(old, new).unwrap();
    assert_eq!(r.summary.original_sentences, 2);
    assert_eq!(r.summary.changed, 1);
    assert_eq!(r.claims[0].line, 3);
}

#[test]
fn run_writes_json_for_a_reviewer_and_text_on_request() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("old.md"), ORIGINAL).unwrap();
    fs::write(dir.path().join("new.md"), REVISED).unwrap();
    let args = |format: &str| {
        claims::command()
            .try_get_matches_from([
                "claims",
                dir.path().join("new.md").to_str().unwrap(),
                "--original",
                dir.path().join("old.md").to_str().unwrap(),
                "--format",
                format,
            ])
            .unwrap()
    };
    let json: serde_json::Value =
        serde_json::from_str(&claims::run(&args("json")).unwrap()).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["summary"]["dropped"], 3);
    assert_eq!(json["claims"][0]["change"], "changed");
    let text = claims::run(&args("text")).unwrap();
    assert!(text.contains("dropped fact line 3: Entries expire after 24 hours."));
    assert!(text.contains("negation changed"));
}

#[test]
fn output_is_deterministic() {
    let a: Report = compare(ORIGINAL, REVISED).unwrap();
    let b: Report = compare(ORIGINAL, REVISED).unwrap();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn indentation_only_change_in_fenced_code_is_a_changed_claim() {
    let old = "```sh\nrun a\n```\n";
    let new = "```sh\n    run a\n```\n";
    let r = compare(old, new).unwrap();
    assert_eq!(r.summary.changed, 1);
    assert!(
        r.claims[0].text.contains("\n    run a\n"),
        "{:?}",
        r.claims[0].text
    );
}

#[test]
fn negations_are_counted_not_flagged() {
    let cases = [
        ("It is not never ready.", "It is never ready.", 2, 1, true),
        ("It is never ready.", "It is not never ready.", 1, 2, true),
        ("It is not ready, not safe.", "It is not ready.", 2, 1, true),
        ("It is not ready.", "It is not ready now.", 1, 1, false),
        ("It is ready.", "It is not ready.", 0, 1, true),
    ];
    for (old, new, before, after, changed) in cases {
        let r = compare(old, new).unwrap();
        assert_eq!(r.claims.len(), 1, "{old}");
        let c = &r.claims[0];
        assert_eq!(
            (
                c.negations_original,
                c.negations_revised,
                c.negation_changed
            ),
            (before, after, changed),
            "{old}"
        );
    }
}

#[test]
fn signed_numbers_are_distinct_literals() {
    let cases = [
        (
            "The offset is -5 pixels.",
            "The offset is 5 pixels.",
            "-5",
            "5",
        ),
        ("The step is +3 pixels.", "The step is 3 pixels.", "+3", "3"),
        (
            "The shift is 5 pixels.",
            "The shift is -5 pixels.",
            "5",
            "-5",
        ),
    ];
    for (old, new, gone, came) in cases {
        let r = compare(old, new).unwrap();
        let c = &r.claims[0];
        assert_eq!(
            kinds(&c.literals_removed),
            [("number".into(), gone.into())],
            "{old}"
        );
        assert_eq!(
            kinds(&c.literals_added),
            [("number".into(), came.into())],
            "{old}"
        );
    }
    let same = compare("The delta is \u{2212}2 units.", "The delta is -2 units.").unwrap();
    assert!(same.claims[0].literals_added.is_empty() && same.claims[0].literals_removed.is_empty());
}

#[test]
fn a_name_the_tokenizer_collapses_in_only_one_sentence_is_still_covered() {
    // "Alice Cooper" opens the original sentence, so slopvac-text keeps it as two words;
    // in the revision it follows other words and collapses into one placeholder.
    let r = compare(
        "Alice Cooper handles requests. The module handles routine requests.",
        "The module handles routine requests for Alice Cooper.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1, "{r:?}");
    assert!(!r.dropped[0].fact_candidate, "{:?}", r.dropped[0]);
}

#[test]
fn a_repeated_collapsed_phrase_does_not_count_as_covered() {
    // The quoted span repeats words ("normal jobs", "unique"). Each occurrence is a
    // token the revision lacks, so coverage is 6/12 and the sentence is a dropped fact.
    let r = compare(
        "The processor serves normal jobs. The processor serves 'normal jobs normal jobs unique unique unique' unique duties separately.",
        "The processor serves normal jobs.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1, "{r:?}");
    assert!(r.dropped[0].fact_candidate, "{:?}", r.dropped[0]);
}

#[test]
fn hyphenated_compounds_in_a_quoted_span_count_as_retained() {
    // The quoted span collapses to a placeholder, so its words are recovered from the text.
    // The revision keeps both compounds whole, so five of the six content words (all but
    // "check") are covered.
    let r = compare(
        "Keep alpha beta gamma. Check 'well-known cache-line' alpha beta gamma.",
        "Keep alpha beta gamma with well-known cache-line.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1, "{r:?}");
    assert_eq!(r.dropped[0].token_coverage, 0.833, "{:?}", r.dropped[0]);
    assert!(!r.dropped[0].fact_candidate, "{:?}", r.dropped[0]);
}

#[test]
fn stopwords_inside_a_collapsed_span_do_not_count_as_content() {
    // The quoted span holds six "this", all stopwords. Coverage counts only the three
    // content words, which the revision keeps, so the sentence is covered.
    let r = compare(
        "Keep alpha beta gamma. Check 'this this this this this this' alpha beta gamma.",
        "Keep alpha beta gamma.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1, "{r:?}");
    assert!(!r.dropped[0].fact_candidate, "{:?}", r.dropped[0]);
}

#[test]
fn doubled_hyphens_in_a_quoted_span_split_like_ordinary_tokens() {
    // The tokenizer keeps a hyphen only with a word character on both sides, so `alpha--beta`
    // is the two words `alpha` and `beta`. The revision keeps both words, so only "check"
    // of the five content words is missing: coverage 4/5 = 0.8.
    let r = compare(
        "Keep gamma delta. Check 'alpha--beta' gamma delta.",
        "Keep gamma delta with alpha beta.",
    )
    .unwrap();
    assert_eq!(r.summary.dropped, 1, "{r:?}");
    assert_eq!(r.dropped[0].token_coverage, 0.8, "{:?}", r.dropped[0]);
}

#[test]
fn a_paragraph_of_very_short_sentences_is_refused_with_the_budget_code() {
    // 300000 sentences of 4 bytes. One sentence for every 8 bytes, plus 65,536, allows
    // 150,000 + 65,536 = 215,536 of them, so the paragraph is not segmented.
    let paragraph = format!("{}\n", "Go. ".repeat(300_000));
    let result = compare("The pack is signed.\n", &paragraph);
    assert!(
        matches!(&result, Err(e) if e.code() == "context-budget-exceeded"),
        "{:?}",
        result.map(|r| r.summary)
    );
}
