//! Corpus check: share of prose sentences that carry each built-in tag kind.
//!
//! Usage: `cargo run -p slopvac-context --example tagstat -- <files...>`
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "example binary: it prints its report and its failure to the terminal"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeMap;
use std::process::ExitCode;

use slopvac_context::{BlockKind, DocumentView, PlainDocument, builtin_registry};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tagstat: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), slopvac_context::Error> {
    let reg = builtin_registry()?;
    let mut total = 0usize;
    let mut per_kind: BTreeMap<String, usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let doc = PlainDocument::from_markdown(&path, &src);
        // A document past the sentence limit is refused here, as it is in a lint run.
        let idx = reg.tag_document(&doc)?;
        for b in doc.blocks() {
            if matches!(b.kind, BlockKind::Heading | BlockKind::CodeBlock) {
                continue;
            }
            for s in doc.sentences(&b)? {
                total += 1;
                for k in idx.kinds_at(&s.span) {
                    *per_kind.entry(k.to_string()).or_default() += 1;
                }
            }
        }
    }
    println!("sentences: {total}");
    for (k, n) in per_kind {
        println!("{k}: {n} ({:.1}%)", 100.0 * n as f64 / total.max(1) as f64);
    }
    Ok(())
}
