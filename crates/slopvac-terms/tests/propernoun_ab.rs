//! The H1 against H4 transformation of stored eval results.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use serde_json::{Value, json};
use slopvac_terms::propernoun::ab::transform;

fn finding(rule: &str, text: &str) -> Value {
    json!({"line": 1, "rule": rule, "severity": "suggestion", "text": text})
}

const RULE: &str = "ai-tells-formatting.title-case-heading";

#[test]
fn only_headings_that_the_repository_body_names_are_dropped() {
    let manifest = json!({"documents": [
        {"id": "a1", "origin": "human", "format": "markdown", "text": "We deploy with Kubernetes today.\n",
         "source": {"type": "github-file", "repo": "org/a"}},
        {"id": "a2", "origin": "human", "format": "markdown", "text": "Nothing to see.\n",
         "source": {"type": "github-file", "repo": "org/a"}},
        {"id": "a3", "origin": "seeded-defect", "format": "markdown", "text": "Seeded.\n",
         "source": {"type": "derived", "parents": ["a1"]}},
        {"id": "b1", "origin": "human", "format": "markdown", "text": "Plain.\n",
         "source": {"type": "github-file", "repo": "org/b"}},
    ]});
    let mut results = json!({"documents": {
        "a1": {"words": 100, "findings": []},
        "a2": {"words": 200, "findings": [
            finding(RULE, "Deploy Kubernetes Safely"),
            finding(RULE, "Install Kubernetes"),
            finding("prose-craft.hyphens", "Install Kubernetes"),
        ]},
        "a3": {"words": 50, "findings": [finding(RULE, "Install Kubernetes")]},
        "b1": {"words": 300, "findings": [finding(RULE, "Install Kubernetes")]},
    }});
    let summary = transform(&manifest, &mut results, &|d| {
        d["text"].as_str().map(str::to_owned)
    });

    let kept = |id: &str| -> Vec<(String, String)> {
        results["documents"][id]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["rule"].as_str().unwrap().to_owned(),
                    f["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    // "Safely" is not a name, so the first finding stays. "Kubernetes" is, in
    // repo a, so "Install Kubernetes" goes there but another rule's finding on
    // the same text stays.
    assert_eq!(
        kept("a2"),
        [
            (RULE.to_owned(), "Deploy Kubernetes Safely".to_owned()),
            (
                "prose-craft.hyphens".to_owned(),
                "Install Kubernetes".to_owned()
            ),
        ]
    );
    // A derived document takes the repository of its parent.
    assert_eq!(kept("a3").len(), 0);
    // Repo b has no such evidence.
    assert_eq!(kept("b1").len(), 1);

    assert_eq!(summary.origins["human"].h1, 3);
    assert_eq!(summary.origins["human"].h4, 2);
    assert_eq!(summary.origins["human"].words, 600);
    assert_eq!(summary.origins["seeded-defect"].h1, 1);
    assert_eq!(summary.origins["seeded-defect"].h4, 0);
    assert_eq!(summary.dropped.len(), 2);
}

fn manifest(docs: &[(&str, &str, &str, Option<&str>)]) -> Value {
    let list: Vec<Value> = docs
        .iter()
        .map(|(id, origin, text, parent)| {
            let source = match parent {
                Some(p) => json!({"type": "derived", "parents": [p]}),
                None => json!({"type": "github-file", "repo": "org/a"}),
            };
            json!({"id": id, "origin": origin, "format": "markdown", "text": text, "source": source})
        })
        .collect();
    json!({"documents": list})
}

/// Number of findings left per document after the transformation.
fn run(manifest: &Value, findings: &[(&str, &str)]) -> Vec<(String, usize)> {
    let docs: serde_json::Map<String, Value> = findings
        .iter()
        .map(|(id, text)| {
            (
                (*id).to_owned(),
                json!({"words": 10, "findings": [finding(RULE, text)]}),
            )
        })
        .collect();
    let mut results = json!({"documents": docs});
    transform(manifest, &mut results, &|d| {
        d["text"].as_str().map(str::to_owned)
    });
    findings
        .iter()
        .map(|(id, _)| {
            (
                (*id).to_owned(),
                results["documents"][*id]["findings"]
                    .as_array()
                    .unwrap()
                    .len(),
            )
        })
        .collect()
}

#[test]
fn capitalised_use_in_a_derived_document_counts_locally_but_not_for_siblings() {
    let m = manifest(&[
        ("a1", "human", "Plain text.\n", None),
        ("a2", "human", "More plain text.\n", None),
        ("d1", "seeded-defect", "We use Redis daily.\n", Some("a1")),
    ]);
    let kept = run(&m, &[("a2", "Install Redis"), ("d1", "Install Redis")]);
    assert_eq!(kept, [("a2".to_owned(), 1), ("d1".to_owned(), 0)]);
}

#[test]
fn lower_case_use_in_a_derived_document_does_not_enter_the_dictionary() {
    // The original text names Kubernetes once. If the derived copy's lower-case
    // use made it a dictionary word, one hit would no longer be enough.
    let m = manifest(&[
        ("a1", "human", "We deploy with Kubernetes today.\n", None),
        ("a2", "human", "More plain text.\n", None),
        ("d1", "seeded-defect", "run kubernetes now\n", Some("a1")),
    ]);
    let kept = run(&m, &[("a2", "Install Kubernetes")]);
    assert_eq!(kept, [("a2".to_owned(), 0)]);
}
