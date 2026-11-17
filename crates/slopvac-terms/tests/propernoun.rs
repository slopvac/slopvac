//! Doc- and repo-level proper-noun inference (H4).

use slopvac_terms::propernoun::{Counts, Dictionary, RepoCounts, infer};

fn counts(text: &str) -> Counts {
    let mut c = Counts::default();
    c.observe(text);
    c
}

fn no_words() -> Vec<String> {
    Vec::new()
}

fn words(c: &Counts, dict: &Dictionary) -> Vec<String> {
    infer(c, dict).words().map(|(w, _)| w.to_owned()).collect()
}

#[test]
fn one_hit_is_enough_for_a_word_outside_the_dictionary() {
    let c = counts("We deploy with Kubernetes today.");
    assert_eq!(words(&c, &Dictionary::default()), ["kubernetes"]);
}

#[test]
fn a_dictionary_word_needs_two_hits() {
    let dict = Dictionary::from_words(["python"]);
    let one = counts("We write the tools in Python.");
    assert_eq!(words(&one, &dict), no_words());
    let two = counts("We write the tools in Python. The tests use Python too.");
    assert_eq!(words(&two, &dict), ["python"]);
}

#[test]
fn the_ratio_must_reach_four_fifths() {
    let dict = Dictionary::from_words(["python"]);
    // 4 capitalised against 1 lower-case: 0.8 passes.
    let pass = counts(
        "Use Python here. Use Python there. Use Python now. Use Python later. Or use python.",
    );
    assert_eq!(words(&pass, &dict), ["python"]);
    // 3 against 1: 0.75 fails.
    let fail = counts("Use Python here. Use Python there. Use Python now. Or use python.");
    assert_eq!(words(&fail, &dict), no_words());
}

#[test]
fn a_sentence_start_is_not_evidence() {
    let c = counts("Docker is popular. Docker helps. Kubernetes follows.");
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_paragraph_break_and_a_list_marker_start_a_sentence() {
    let c =
        counts("Intro text\n\nDocker first.\n\n- Docker second\n1. Docker third\n> Docker fourth");
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_wrapped_line_continues_the_sentence() {
    let c = counts("We run everything on\nKubernetes in production.");
    assert_eq!(words(&c, &Dictionary::default()), ["kubernetes"]);
}

#[test]
fn setext_headings_are_not_body_evidence() {
    let c = counts(
        "Gold Sponsors\n=============\n\nCompatibility Notes\n-------------------\n\nSome text.\n",
    );
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn headings_fences_and_code_spans_are_not_body_evidence() {
    let c = counts(
        "# Using Docker Desktop\n\n```sh\nrun Docker now\n```\n\nCall `Foo Bar` then see [here](https://Example.com/Zed).\n",
    );
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_word_seen_lower_case_often_is_not_a_proper_noun() {
    let c = counts(
        "We keep the Desktop clean. The desktop is fast. A desktop app. The desktop shell. My desktop.",
    );
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_phrase_is_evidence_of_its_own_when_its_words_are_ordinary() {
    let dict = Dictionary::from_words(["test", "plan"]);
    let c = counts(
        "Read the Test Plan first. Update the Test Plan later. We test the code. We test it. \
         A test passes. A test fails. Make a plan. Our plan works. The plan holds.",
    );
    let found = infer(&c, &dict);
    assert_eq!(
        found.phrases().map(|(p, _)| p).collect::<Vec<_>>(),
        ["test plan"]
    );
    assert_eq!(found.words().count(), 0);
}

#[test]
fn mask_covers_the_longest_chain_of_phrases_in_a_heading() {
    let dict = Dictionary::from_words(["claude", "code", "review"]);
    let c = counts("We ask Claude Code Review to help. Later Claude Code Review ran again.");
    let found = infer(&c, &dict);
    let heading = "Run Claude Code Review Now";
    let spans: Vec<&str> = found
        .mask(heading)
        .iter()
        .map(|r| &heading[r.clone()])
        .collect();
    assert_eq!(spans, ["Claude", "Code", "Review"]);
}

#[test]
fn mask_covers_inferred_single_words_and_leaves_the_rest() {
    let c = counts("We deploy with Kubernetes today.");
    let found = infer(&c, &Dictionary::default());
    let heading = "Deploy Kubernetes Safely";
    let spans: Vec<&str> = found
        .mask(heading)
        .iter()
        .map(|r| &heading[r.clone()])
        .collect();
    assert_eq!(spans, ["Kubernetes"]);
}

#[test]
fn counts_merge_like_a_single_pass() {
    let parts = [
        "We deploy with Kubernetes today.",
        "We like Python.",
        "We use Python daily.",
    ];
    let dict = Dictionary::from_words(["python"]);
    let mut merged = Counts::default();
    for p in parts {
        merged.merge(&counts(p));
    }
    let whole = counts(&parts.join("\n\n"));
    assert_eq!(words(&merged, &dict), ["kubernetes", "python"]);
    assert_eq!(words(&whole, &dict), ["kubernetes", "python"]);
}

#[test]
fn dictionary_comes_from_lower_case_use() {
    let c = counts("Run the desktop app. Use a desktop.");
    let dict = Dictionary::from_counts(&c);
    assert!(dict.contains("desktop"));
    assert!(!dict.contains("kubernetes"));
}

#[test]
fn repo_counts_skip_unchanged_files_and_replace_changed_ones() {
    let mut repo = RepoCounts::default();
    assert!(repo.update("a.md", "We deploy with Kubernetes today."));
    assert!(!repo.update("a.md", "We deploy with Kubernetes today."));
    assert_eq!(words(&repo.total(), &Dictionary::default()), ["kubernetes"]);
    assert!(repo.update("a.md", "We deploy with nothing today."));
    assert_eq!(words(&repo.total(), &Dictionary::default()), no_words());
}

#[test]
fn repo_counts_survive_a_json_round_trip() {
    let mut repo = RepoCounts::default();
    repo.update("a.md", "We deploy with Kubernetes today.");
    let json = serde_json::to_string(&repo).unwrap();
    let back: RepoCounts = serde_json::from_str(&json).unwrap();
    assert_eq!(back, repo);
}

#[test]
fn repo_counts_drop_a_removed_file() {
    let mut repo = RepoCounts::default();
    repo.update("a.md", "We deploy with Kubernetes today.");
    assert!(repo.remove("a.md"));
    assert_eq!(words(&repo.total(), &Dictionary::default()), no_words());
}

#[test]
fn residual_capitals_count_title_case_words_that_no_name_explains() {
    let c = counts("We deploy with Kubernetes today. We also run Docker Desktop daily.");
    let found = infer(&c, &Dictionary::default());
    // The first word is never counted: sentence case capitalises it.
    assert_eq!(found.residual_capitals("Deploy Kubernetes Safely"), 1);
    assert_eq!(found.residual_capitals("Install Docker Desktop"), 0);
    assert_eq!(found.residual_capitals("Configure The Server"), 2);
    assert_eq!(found.residual_capitals("Using the Kubernetes API"), 0);
}

#[test]
fn echoes_of_headings_are_not_body_evidence() {
    let c = counts(
        "Contents\n\n- [Source Code](#source-code)\n  * [Client Binaries](#client-binaries)\n\n\
         !!! note \"Technical Details\"\n\n**Compatibility Notes**\n\n__Gold Sponsors:__\n",
    );
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_link_to_another_page_is_body_evidence() {
    let c =
        counts("See the docs for Kubernetes [here](https://example.com/x) and [Redis](other.md).");
    assert_eq!(words(&c, &Dictionary::default()), ["kubernetes", "redis"]);
}

#[test]
fn a_pair_that_starts_a_sentence_is_not_evidence() {
    let c = counts("Claude Code is fine. Claude Code works.");
    let found = infer(&c, &Dictionary::default());
    assert_eq!(found.phrases().count(), 0);
}

#[test]
fn a_pair_after_a_sentence_start_word_still_counts_from_its_second_word() {
    let c = counts("Install Docker Desktop now.");
    let found = infer(&c, &Dictionary::default());
    assert_eq!(
        found.phrases().map(|(p, _)| p).collect::<Vec<_>>(),
        ["docker desktop"]
    );
}

#[test]
fn front_matter_is_not_body_evidence() {
    let c = counts("---\ntitle: We use Kubernetes here\n---\n\nPlain text.\n");
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn headings_inside_quotes_and_lists_are_not_body_evidence() {
    let c = counts("> # Using Kubernetes now\n\n- ## Docker Desktop here\n");
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_double_backtick_span_may_hold_a_backtick() {
    let c = counts("Call ``Foo `Bar` Baz`` then stop.");
    assert_eq!(words(&c, &Dictionary::default()), no_words());
}

#[test]
fn a_fence_closes_only_with_its_own_character_and_length() {
    let tilde = counts("~~~\nRun Kubernetes\n```\nmore Kubernetes\n~~~\n");
    assert_eq!(words(&tilde, &Dictionary::default()), no_words());
    let long = counts("````\n```\nRun Kubernetes\n```\n````\n\nAfter we run Redis.\n");
    assert_eq!(words(&long, &Dictionary::default()), ["redis"]);
}

#[test]
fn a_heading_that_starts_with_code_has_its_first_word_after_the_code() {
    let c = counts("We use Docker daily.");
    let found = infer(&c, &Dictionary::default());
    assert_eq!(found.residual_capitals("Install Docker"), 0);
    assert_eq!(found.residual_capitals("`run` Install Docker"), 1);
}

#[test]
fn combining_marks_stay_attached_to_their_word() {
    let c = counts("We use Cafe\u{301} daily and Kubernetes too.");
    assert_eq!(words(&c, &Dictionary::default()), ["kubernetes"]);
}

#[test]
fn the_first_token_is_everything_up_to_the_first_space() {
    let c = counts("We use Redis daily.");
    let found = infer(&c, &Dictionary::default());
    // "Source" sits inside the first token, so it is exempt.
    assert_eq!(found.residual_capitals("Open-Source Redis"), 0);
    // A lone symbol is the first token, so "Configure" is not exempt.
    assert_eq!(found.residual_capitals("& Configure Redis"), 1);
    // A code span that holds a space is one token.
    assert_eq!(found.residual_capitals("`a b` Install Redis"), 1);
}
