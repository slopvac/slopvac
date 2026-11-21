//! Count the tags and pieces of tags the engine builds for each document.
//!
//! `cargo run -p slopvac-engine --example tag_density -- <file>...` lints each file as
//! Markdown and prints one line per file: bytes, tags and pieces, and the outcome. The count
//! sums the tags the taggers push and the pieces the table-cell split pushes, so it is an
//! upper bound on the entries the index holds. Measurement tooling for the tag ceiling, not a
//! public interface.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]
#![allow(
    clippy::print_stdout,
    reason = "example binary: it prints its report to the terminal"
)]

use std::process::ExitCode;

use slopvac_context::take_materialised_tags;
use slopvac_engine::{Engine, Input, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString};
use slopvac_rules::RuleSet;

fn main() -> ExitCode {
    let settings = Settings::new(
        NonEmptyString::new("normal").expect("non-empty"),
        GenreResult {
            id: GenreId::new("reference").expect("genre id"),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    let engine = Engine::new(Vec::<RuleSet>::new(), settings).expect("engine");
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{path}\t0\t0\tunreadable");
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            println!("{path}\t0\t0\tnot-utf8");
            continue;
        };
        let _ = take_materialised_tags();
        let outcome = match engine.lint(&Input::markdown(&path, &text)) {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        };
        println!(
            "{path}\t{}\t{}\t{outcome}",
            text.len(),
            take_materialised_tags()
        );
    }
    ExitCode::SUCCESS
}
