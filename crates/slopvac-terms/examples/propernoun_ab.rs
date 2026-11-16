//! A/B for title-case-heading: H1 against H4.
//!
//! H1 is the rule as it ships: it counts every capitalised word of the heading
//! that a fixed shape check does not exempt. H4 also exempts the words that
//! proper-noun inference names from the body prose of the same repository.
//!
//! The example reads the stored results of an engine run (arm A), re-judges
//! each `title-case-heading` finding with the inference and writes arm B in the
//! same format, so `python -m evals ab` can compare them. It also prints the
//! dropped findings, so a person can check that each one is a name.
//!
//! ```sh
//! cargo run -p slopvac-terms --example propernoun_ab -- \
//!     a.json evals/manifest.json evals b.json [extra-cache-dir]
//! ```
//!
//! The inference sees only committed corpus text plus, when given, documents
//! under `<cache>/docs/<id>.txt`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use serde_json::Value;
use slopvac_terms::propernoun::{Counts, Dictionary, infer};

const RULE: &str = "title-case-heading";

fn text_of(evals: &Path, cache: Option<&Path>, doc: &Value) -> Option<String> {
    let id = doc["id"].as_str()?;
    if doc["committed"].as_bool() == Some(true) {
        return fs::read_to_string(evals.join(doc["path"].as_str()?)).ok();
    }
    fs::read_to_string(cache?.join("docs").join(format!("{id}.txt"))).ok()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [results_path, manifest_path, evals, out_path, rest @ ..] = args.as_slice() else {
        eprintln!("usage: propernoun_ab RESULTS MANIFEST EVALS_DIR OUT [CACHE]");
        std::process::exit(2);
    };
    let evals = PathBuf::from(evals);
    let cache = rest.first().map(PathBuf::from);
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
    let mut results: Value =
        serde_json::from_str(&fs::read_to_string(results_path).unwrap()).unwrap();

    // Counts per document, pooled per repository; the dictionary pools all.
    let mut per_doc: BTreeMap<String, Counts> = BTreeMap::new();
    let mut per_repo: BTreeMap<String, Counts> = BTreeMap::new();
    let mut everything = Counts::default();
    let mut origin: BTreeMap<String, String> = BTreeMap::new();
    let mut repo_of: BTreeMap<String, String> = BTreeMap::new();
    for doc in manifest["documents"].as_array().unwrap() {
        let id = doc["id"].as_str().unwrap().to_owned();
        origin.insert(id.clone(), doc["origin"].as_str().unwrap().to_owned());
        let repo = doc["source"]["repo"].as_str().unwrap_or(&id).to_owned();
        repo_of.insert(id.clone(), repo.clone());
        let is_prose = matches!(
            doc["format"].as_str(),
            Some("markdown" | "mermaid-markdown" | "pull-request" | "commit-message")
        );
        if !is_prose {
            continue;
        }
        let Some(text) = text_of(&evals, cache.as_deref(), doc) else {
            continue;
        };
        let mut c = Counts::default();
        c.observe(&text);
        per_repo.entry(repo).or_default().merge(&c);
        everything.merge(&c);
        per_doc.insert(id, c);
    }
    let dict = Dictionary::from_counts(&everything);
    let repo_inferred: BTreeMap<_, _> = per_repo
        .iter()
        .map(|(r, c)| (r.clone(), infer(c, &dict)))
        .collect();

    let mut kept: BTreeMap<String, usize> = BTreeMap::new();
    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    let mut words: BTreeMap<String, u64> = BTreeMap::new();
    let mut shown = Vec::new();
    for (id, res) in results["documents"].as_object_mut().unwrap() {
        let o = origin.get(id).cloned().unwrap_or_default();
        *words.entry(o.clone()).or_default() += res["words"].as_u64().unwrap_or(0);
        let repo = &repo_of[id];
        let doc_inferred = per_doc.get(id).map(|c| infer(c, &dict));
        let findings = res["findings"].as_array_mut().unwrap();
        findings.retain(|f| {
            if !f["rule"].as_str().unwrap().ends_with(RULE) {
                return true;
            }
            let heading = f["text"].as_str().unwrap();
            let explained =
                |inf: &slopvac_terms::propernoun::Inferred| inf.residual_capitals(heading);
            let residual = [repo_inferred.get(repo), doc_inferred.as_ref()]
                .into_iter()
                .flatten()
                .map(explained)
                .min()
                .unwrap_or(usize::MAX);
            if residual == 0 {
                *dropped.entry(o.clone()).or_default() += 1;
                shown.push(format!("{o}\t{repo}\t{heading}"));
                false
            } else {
                *kept.entry(o.clone()).or_default() += 1;
                true
            }
        });
    }
    shown.sort();
    for s in &shown {
        println!("DROPPED\t{s}");
    }
    for (o, w) in &words {
        let k = kept.get(o).copied().unwrap_or(0);
        let d = dropped.get(o).copied().unwrap_or(0);
        println!("SUMMARY\t{o}\twords={w}\tH1={}\tH4={k}\tdropped={d}", k + d);
    }
    results["engine"]["name"] = Value::String("h4-propernoun".into());
    fs::write(out_path, serde_json::to_string(&results).unwrap()).unwrap();
}
