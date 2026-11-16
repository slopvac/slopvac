//! Lint diagram samples with the diagram rule pack and print the findings as JSON.
//!
//! `cargo run -p slopvac-engine --example diagram_eval -- <file>...` reads Markdown
//! (`.md`) and diagram files (`.mmd`, `.puml`, `.drawio`, ...) and prints one JSON object
//! per file: `{"file", "findings": [{"rule", "line", "matched"}]}`. `evals/diagrams.py`
//! scores that output against the hand labels. This is eval tooling, not a public interface.

use std::process::ExitCode;

use serde_json::json;
use slopvac_diagrams::Format;
use slopvac_engine::{DIAGRAM_PACK, Engine, Input, Settings, diagram_rule_set};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, PackId};

fn settings() -> Settings {
    Settings::new(
        NonEmptyString::new("normal").expect("non-empty"),
        GenreResult {
            id: GenreId::new("reference").expect("genre id"),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    )
}

fn main() -> ExitCode {
    let pack = PackId::new(DIAGRAM_PACK).expect("pack id");
    let set = match diagram_rule_set(&pack) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let engine = match Engine::new(vec![set], settings()) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut out = Vec::new();
    for path in std::env::args().skip(1) {
        let source = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        };
        let is_markdown = std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        let input = if is_markdown {
            Input::markdown(&path, &source)
        } else if let Some(format) = Format::from_path(&path) {
            Input::diagram(&path, &source, format)
        } else {
            eprintln!("{path}: not a Markdown or diagram file");
            return ExitCode::from(2);
        };
        let doc = match engine.lint(&input) {
            Ok(doc) => doc,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        };
        let findings: Vec<_> = doc
            .findings
            .iter()
            .map(|f| {
                let id = f.rule_id.as_str();
                let rule = id.split_once(':').map_or(id, |(_, local)| local);
                json!({"rule": rule, "line": f.start.line(), "matched": f.matched_text})
            })
            .collect();
        out.push(json!({"file": path, "findings": findings}));
    }
    println!("{}", serde_json::Value::Array(out));
    ExitCode::SUCCESS
}
