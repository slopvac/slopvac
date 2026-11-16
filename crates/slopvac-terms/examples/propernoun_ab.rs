//! A/B for title-case-heading: H1 against H4.
//!
//! Reads the stored results of an engine run (arm A), drops the
//! `title-case-heading` findings that the proper-noun inference explains (see
//! `slopvac_terms::propernoun::ab`) and writes arm B in the same format, so
//! `python -m evals ab` can compare them. It prints the dropped findings so a
//! person can check that each one is a name.
//!
//! ```sh
//! cargo run -p slopvac-terms --example propernoun_ab -- \
//!     a.json evals/manifest.json evals b.json [extra-cache-dir]
//! ```
//!
//! The inference sees committed corpus text plus, when given, documents under
//! `<cache>/docs/<id>.txt`.

use std::path::PathBuf;
use std::{env, fs};

use serde_json::Value;
use slopvac_terms::propernoun::ab::transform;

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

    let read = |doc: &Value| -> Option<String> {
        let id = doc["id"].as_str()?;
        if doc["committed"].as_bool() == Some(true) {
            return fs::read_to_string(evals.join(doc["path"].as_str()?)).ok();
        }
        fs::read_to_string(cache.as_ref()?.join("docs").join(format!("{id}.txt"))).ok()
    };
    let summary = transform(&manifest, &mut results, &read);

    let mut shown: Vec<String> = summary
        .dropped
        .iter()
        .map(|d| format!("{}\t{}\t{}", d.origin, d.repo, d.heading))
        .collect();
    shown.sort();
    for s in &shown {
        println!("DROPPED\t{s}");
    }
    for (o, s) in &summary.origins {
        println!(
            "SUMMARY\t{o}\twords={}\tH1={}\tH4={}\tdropped={}",
            s.words,
            s.h1,
            s.h4,
            s.h1 - s.h4
        );
    }
    results["engine"]["name"] = Value::String("h4-propernoun".into());
    fs::write(out_path, serde_json::to_string(&results).unwrap()).unwrap();
}
