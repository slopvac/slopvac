//! Write the error-code reference page under a directory.
//!
//! `cargo run -p slopvac --example reference -- <out-dir>` writes `reference/error-codes.md`
//! below `<out-dir>`. It lists every code in [`slopvac_cli::messages::ALL`] and every retired
//! code.

use std::fmt::Write as _;
use std::path::PathBuf;

use slopvac_cli::messages::{self, Retired};
use slopvac_messages::{Entry, Kind, ParamKind};

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: reference <out-dir>"));
    let file = out.join("reference/error-codes.md");
    std::fs::create_dir_all(file.parent().expect("page path has a directory"))?;
    std::fs::write(file, page(&messages::catalogs(), messages::RETIRED))
}

/// The error-code page: one table per catalog, then the retired codes.
pub(crate) fn page(catalogs: &[(&str, &[Entry])], retired: &[Retired]) -> String {
    let mut out = String::from(
        "# Error codes\n\n\
         Every message the command line prints has a stable code. A code never changes meaning \
         and is never reused. Message text can change in any release, so match on the code.\n\n",
    );
    // Text ids are not codes: the page lists errors and notices only.
    let listed: Vec<(&str, Vec<&Entry>)> = catalogs
        .iter()
        .map(|(name, entries)| {
            let kept = entries.iter().filter(|e| e.kind != Kind::Text).collect();
            (*name, kept)
        })
        .filter(|(_, kept): &(&str, Vec<&Entry>)| !kept.is_empty())
        .collect();
    if listed.is_empty() {
        out.push_str("No codes are registered yet.\n");
    }
    for (name, entries) in &listed {
        let _ = writeln!(out, "## {name}\n");
        out.push_str(
            "| Code | Kind | Message | Parameters | Summary |\n| --- | --- | --- | --- | --- |\n",
        );
        for entry in entries {
            let params = entry
                .params
                .iter()
                .map(|p| match p.kind {
                    ParamKind::Count => format!("{} (count)", cell(p.name)),
                    ParamKind::Path => format!("{} (path)", cell(p.name)),
                    _ => cell(p.name),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} |",
                entry.code,
                entry.kind.name(),
                code_cell(entry.template),
                if params.is_empty() {
                    "none".to_owned()
                } else {
                    params
                },
                cell(entry.summary),
            );
        }
        out.push('\n');
    }
    if !retired.is_empty() {
        out.push_str("## Retired codes\n\nThese codes no longer appear. Do not reuse them.\n\n");
        out.push_str("| Code | Note |\n| --- | --- |\n");
        for r in retired {
            let _ = writeln!(out, "| `{}` | {} |", r.code, cell(r.note));
        }
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// Text for a table cell: one line, with the characters that end a cell escaped.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

/// A template as a code span. The fence is one backtick longer than any run inside.
fn code_cell(text: &str) -> String {
    let body = cell(text);
    let (mut longest, mut run) = (0, 0);
    for c in body.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let pad = if body.starts_with('`') || body.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{body}{pad}{fence}")
}
