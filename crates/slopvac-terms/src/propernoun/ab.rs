//! The H1 against H4 transformation of stored eval results.
//!
//! H1 is the `title-case-heading` rule as it ships. H4 also drops a finding
//! when every capitalised word after the heading's first word is named by the
//! proper-noun inference of the document's repository or of the document.
//! Repository evidence comes from original documents only: a derived document
//! (seeded defect, control, rewrite) copies its parent, so it would count the
//! parent's text twice. A derived document still takes the repository of its
//! parent.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Counts, Dictionary, Inferred, infer};

const RULE: &str = "title-case-heading";

/// Finding counts for one origin class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OriginSummary {
    /// Words in the documents that have results.
    pub words: u64,
    /// `title-case-heading` findings before the transformation.
    pub h1: usize,
    /// Findings left after it.
    pub h4: usize,
}

/// A finding that H4 removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    /// Origin class of the document.
    pub origin: String,
    /// Repository the document belongs to.
    pub repo: String,
    /// The heading text.
    pub heading: String,
}

/// What the transformation did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Counts per origin class.
    pub origins: BTreeMap<String, OriginSummary>,
    /// Every removed finding.
    pub dropped: Vec<Dropped>,
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// The repository of a manifest record: its own, or its first parent's.
fn repo_of(docs: &BTreeMap<&str, &Value>, id: &str) -> String {
    let mut cur = id;
    for _ in 0..16 {
        let Some(doc) = docs.get(cur) else { break };
        if let Some(repo) = doc["source"]["repo"].as_str() {
            return repo.to_owned();
        }
        match doc["source"]["parents"][0].as_str() {
            Some(parent) => cur = parent,
            None => break,
        }
    }
    id.to_owned()
}

/// Drop the `title-case-heading` findings in `results` that H4 explains.
///
/// `read` returns the text of a manifest record, or `None` when it is not
/// available. Only prose records are read.
pub fn transform(
    manifest: &Value,
    results: &mut Value,
    read: &dyn Fn(&Value) -> Option<String>,
) -> Summary {
    let empty = Vec::new();
    let records = manifest["documents"].as_array().unwrap_or(&empty);
    let docs: BTreeMap<&str, &Value> = records.iter().map(|d| (str_of(d, "id"), d)).collect();

    let mut per_doc: BTreeMap<String, Counts> = BTreeMap::new();
    let mut per_repo: BTreeMap<String, Counts> = BTreeMap::new();
    let mut everything = Counts::default();
    for doc in records {
        let prose = matches!(
            str_of(doc, "format"),
            "markdown" | "mermaid-markdown" | "pull-request" | "commit-message"
        );
        let Some(text) = prose.then(|| read(doc)).flatten() else {
            continue;
        };
        let id = str_of(doc, "id");
        let mut c = Counts::default();
        c.observe(&text);
        if str_of(&doc["source"], "type") != "derived" {
            per_repo.entry(repo_of(&docs, id)).or_default().merge(&c);
            everything.merge(&c);
        }
        per_doc.insert(id.to_owned(), c);
    }
    let dict = Dictionary::from_counts(&everything);
    let repo_inferred: BTreeMap<&String, Inferred> =
        per_repo.iter().map(|(r, c)| (r, infer(c, &dict))).collect();

    let mut summary = Summary::default();
    let Some(entries) = results["documents"].as_object_mut() else {
        return summary;
    };
    for (id, res) in entries {
        let origin = docs.get(id.as_str()).map_or("", |d| str_of(d, "origin"));
        let repo = repo_of(&docs, id);
        let stats = summary.origins.entry(origin.to_owned()).or_default();
        stats.words += res["words"].as_u64().unwrap_or(0);
        let doc_inferred = per_doc.get(id).map(|c| infer(c, &dict));
        let Some(findings) = res["findings"].as_array_mut() else {
            continue;
        };
        findings.retain(|f| {
            if !str_of(f, "rule").ends_with(RULE) {
                return true;
            }
            stats.h1 += 1;
            let heading = str_of(f, "text");
            let residual = [repo_inferred.get(&repo), doc_inferred.as_ref()]
                .into_iter()
                .flatten()
                .map(|inf| inf.residual_capitals(heading))
                .min()
                .unwrap_or(usize::MAX);
            if residual == 0 {
                summary.dropped.push(Dropped {
                    origin: origin.to_owned(),
                    repo: repo.clone(),
                    heading: heading.to_owned(),
                });
                false
            } else {
                stats.h4 += 1;
                true
            }
        });
    }
    summary
}
