//! Corpus check: share of prose sentences that carry each built-in tag kind.
//!
//! Usage: `cargo run -p slopvac-context --example tagstat -- <files...>`

use std::collections::BTreeMap;

use slopvac_context::{BlockKind, DocumentView, PlainDocument, builtin_registry};

#[expect(
    clippy::print_stdout,
    reason = "example binary reports on the standard streams"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reg = builtin_registry()?;
    let mut total = 0usize;
    let mut per_kind: BTreeMap<String, usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let doc = PlainDocument::from_markdown(&path, &src);
        let idx = reg.tag_document(&doc)?;
        for b in doc.blocks() {
            if matches!(b.kind, BlockKind::Heading | BlockKind::CodeBlock) {
                continue;
            }
            for s in doc.sentences(&b) {
                total += 1;
                for k in idx.kinds_at(&s.span) {
                    *per_kind.entry(k.to_owned()).or_default() += 1;
                }
            }
        }
    }
    println!("sentences: {total}");
    let total_f = f64::from(u32::try_from(total.max(1))?);
    for (k, n) in per_kind {
        println!(
            "{k}: {n} ({:.1}%)",
            100.0 * f64::from(u32::try_from(n)?) / total_f
        );
    }
    Ok(())
}
