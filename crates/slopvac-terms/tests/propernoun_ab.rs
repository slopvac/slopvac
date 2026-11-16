//! The H1 against H4 transformation of stored eval results.

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
