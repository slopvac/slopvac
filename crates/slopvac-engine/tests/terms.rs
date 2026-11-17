//! The term-casing rule: `terms.casing` over the term set of `slopvac-terms`.
//!
//! Expected spellings are worked out from the canonical terms (`GitHub`, `JavaScript`), not
//! from the matcher: a term differs from the canonical spelling in case only.
#![cfg(test)]

#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
)]
mod common;

use common::{matched, places, settings};
use slopvac_engine::{Engine, Input, TERMS_PACK, terms_rule_set};
use slopvac_model::{Document, PackId};
use slopvac_terms::TermSet;

fn engine_with(allowed: &[&str]) -> Engine {
    let set = terms_rule_set(&PackId::new(TERMS_PACK).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    let mut settings = settings();
    settings.allowed_words = allowed.iter().map(|w| (*w).to_owned()).collect();
    Engine::new(vec![set], settings).unwrap()
}

fn lint(text: &str) -> Document {
    engine_with(&[])
        .lint(&Input::markdown("a.md", text))
        .unwrap()
}

#[test]
fn a_miscased_term_is_a_finding_with_the_canonical_fix() {
    let doc = lint("Open the repository on Github.\n");
    assert_eq!(
        places(&doc),
        [("terms:terms.casing".to_owned(), 1, 24)],
        "{doc:?}"
    );
    let finding = &doc.findings[0];
    assert_eq!(finding.matched_text, "Github");
    assert_eq!(
        finding.message.as_str(),
        "\"Github\" is not the canonical spelling: write \"GitHub\"."
    );
    let edits = finding.fix.as_ref().unwrap().edits();
    assert_eq!(edits.len(), 1);
    // "Open the repository on " is 23 bytes.
    assert_eq!((edits[0].span.start(), edits[0].span.end()), (23, 29));
    assert_eq!(edits[0].text, "GitHub");
}

#[test]
fn the_canonical_spelling_is_not_a_finding() {
    assert_eq!(
        matched(&lint("Open the repository on GitHub.\n")),
        Vec::<&str>::new()
    );
}

#[test]
fn the_rule_examples_flag_bad_text_with_the_good_text_as_its_fix() {
    let set = terms_rule_set(&PackId::new(TERMS_PACK).unwrap()).unwrap();
    let rule = set.rules().next().unwrap();
    assert_ne!(rule.rule().examples(), []);
    let engine = engine_with(&[]);
    for example in rule.rule().examples() {
        let bad = format!("{}\n", example.bad);
        let doc = engine.lint(&Input::markdown("a.md", &bad)).unwrap();
        assert!(!doc.findings.is_empty(), "{}", example.bad);
        let mut fixed = bad.clone();
        for f in doc.findings.iter().rev() {
            let e = &f.fix.as_ref().unwrap().edits()[0];
            fixed.replace_range(e.span.start()..e.span.end(), &e.text);
        }
        let good = example.good.as_ref().unwrap();
        assert_eq!(fixed.trim_end(), good);
        let clean = format!("{good}\n");
        assert_eq!(
            engine
                .lint(&Input::markdown("a.md", &clean))
                .unwrap()
                .findings
                .len(),
            0,
            "{good}"
        );
    }
}

#[test]
fn a_term_in_code_is_not_a_finding() {
    let cases = [
        "Run `Github` here.\n",
        "```\nGithub\n```\n",
        "~~~sh\ngit clone https://Github.com/a/b\n~~~\n",
        "    Github\n\nText.\n",
    ];
    for text in cases {
        assert_eq!(matched(&lint(text)), Vec::<&str>::new(), "{text:?}");
    }
}

#[test]
fn prose_around_a_code_span_is_still_checked() {
    let doc = lint("Run `Github` on Github.\n");
    assert_eq!(matched(&doc), ["Github"]);
    // "Run `Github` on " is 16 bytes.
    assert_eq!(doc.findings[0].start.column(), 17);
}

#[test]
fn front_matter_is_not_checked() {
    assert_eq!(
        matched(&lint("---\ntitle: Github\n---\n\nPlain text.\n")),
        Vec::<&str>::new()
    );
}

#[test]
fn an_allowed_word_is_not_a_finding() {
    let doc = engine_with(&["github"])
        .lint(&Input::markdown("a.md", "See Github and Javascript.\n"))
        .unwrap();
    assert_eq!(matched(&doc), ["Javascript"]);
}

#[test]
fn a_disable_directive_silences_the_rule() {
    let text = "<!-- slopvac-disable-next-line -->\nSee Github.\n";
    assert_eq!(matched(&lint(text)), Vec::<&str>::new(), "{text:?}");
}

#[test]
fn an_empty_term_set_flags_nothing() {
    let engine = engine_with(&[]).with_terms(TermSet::default());
    let doc = engine
        .lint(&Input::markdown("a.md", "See Github.\n"))
        .unwrap();
    assert_eq!(doc.findings.len(), 0);
}

#[test]
fn the_finding_is_in_the_serialized_report() {
    let doc = lint("See Github.\n");
    let json = serde_json::to_value(&doc).unwrap();
    let findings = json["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["rule_id"], "terms:terms.casing");
    assert_eq!(findings[0]["matched_text"], "Github");
}

#[test]
fn the_rule_is_executed_by_the_engine() {
    let engine = engine_with(&[]);
    let set = terms_rule_set(&PackId::new(TERMS_PACK).unwrap()).unwrap();
    let rule = set.rules().next().unwrap();
    assert_eq!(engine.unexecuted(rule), None);
}
