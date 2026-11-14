//! The `slopvac terms` command: list, search, add, remove, check and import.
//!
//! `command()` returns the clap tree and `run()` executes a parsed match. The
//! CLI crate mounts them; nothing here owns `main`.

use std::fmt::Write as _;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::edit::{TermsDoc, infer_ignore_case};
use crate::import::{self, Source};
use crate::{TermEntry, TermSet, TermsError, TermsFile};

const DEFAULT_FILE: &str = ".slopvac/terms.toml";

fn files_arg() -> Arg {
    Arg::new("files")
        .long("files")
        .value_name("PATH")
        .action(ArgAction::Append)
        .help("Terms file (default: .slopvac/terms.toml)")
}

/// The `terms` command tree.
#[must_use]
pub fn command() -> Command {
    Command::new("terms")
        .about("Manage project terms")
        .subcommand_required(true)
        .subcommand(
            Command::new("list")
                .about("List terms")
                .arg(Arg::new("kind").long("kind").value_name("KIND"))
                .arg(
                    Arg::new("builtin")
                        .long("builtin")
                        .action(ArgAction::SetTrue)
                        .help("Include the built-in list"),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("search")
                .about("Find terms by substring, ignoring case")
                .arg(Arg::new("query").required(true))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("add")
                .about("Add a term; lower-case terms default to ignoring case")
                .arg(Arg::new("term").required(true))
                .arg(Arg::new("kind").long("kind").value_name("KIND"))
                .arg(
                    Arg::new("variant")
                        .long("variant")
                        .value_name("SPELLING")
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("ignore-case")
                        .long("ignore-case")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("case-sensitive"),
                )
                .arg(
                    Arg::new("case-sensitive")
                        .long("case-sensitive")
                        .action(ArgAction::SetTrue),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("remove")
                .about("Remove a term")
                .arg(Arg::new("term").required(true))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("check")
                .about("Report wrong-case terms in files; exit 1 when any are found")
                .arg(Arg::new("paths").required(true).num_args(1..))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("import")
                .about("Import words from another tool")
                .arg(Arg::new("from").long("from").required(true).value_parser([
                    "vale",
                    "cspell",
                    "codespell",
                ]))
                .arg(Arg::new("file").required(true))
                .arg(files_arg()),
        )
}

fn term_files(m: &ArgMatches, cwd: &Path) -> Vec<PathBuf> {
    let given: Vec<PathBuf> = m
        .get_many::<String>("files")
        .map(|v| v.map(|s| cwd.join(s)).collect())
        .unwrap_or_default();
    if given.is_empty() {
        vec![cwd.join(DEFAULT_FILE)]
    } else {
        given
    }
}

fn load_project(paths: &[PathBuf]) -> Result<Vec<TermEntry>, TermsError> {
    let mut out = Vec::new();
    for p in paths {
        if p.exists() {
            out.extend(TermsFile::load(p)?.terms);
        }
    }
    Ok(out)
}

fn io_err(e: &std::io::Error) -> TermsError {
    TermsError::Io {
        path: "output".to_owned(),
        message: e.to_string(),
    }
}

fn line_col(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map_or(before, |i| &before[i + 1..]);
    (line, col.chars().count() + 1)
}

/// Byte ranges of fenced code blocks and inline code spans in Markdown.
#[must_use]
pub fn markdown_code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fence: Option<usize> = None;
    let mut spans: Vec<(usize, &str)> = Vec::new();
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            match fence.take() {
                Some(start) => out.push(start..offset + line.len()),
                None => fence = Some(offset),
            }
        } else if fence.is_none() {
            spans.push((offset, line));
        }
        offset += line.len();
    }
    if let Some(start) = fence {
        out.push(start..text.len());
    }
    for (base, line) in spans {
        let mut open: Option<usize> = None;
        for (i, c) in line.char_indices() {
            if c == '`' {
                match open.take() {
                    Some(s) => out.push(base + s..base + i + 1),
                    None => open = Some(i),
                }
            }
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Run a parsed `terms` command. Returns the process exit code.
///
/// # Errors
/// File, parse and validation errors.
pub fn run(matches: &ArgMatches, cwd: &Path, out: &mut dyn Write) -> Result<i32, TermsError> {
    let Some((name, m)) = matches.subcommand() else {
        return Ok(2);
    };
    let paths = term_files(m, cwd);
    match name {
        "list" => {
            let kind = m.get_one::<String>("kind");
            let mut rows: Vec<(TermEntry, &str)> = load_project(&paths)?
                .into_iter()
                .map(|t| (t, "project"))
                .collect();
            if m.get_flag("builtin") {
                rows.extend(
                    TermsFile::builtin()
                        .terms
                        .into_iter()
                        .map(|t| (t, "builtin")),
                );
            }
            for (t, src) in rows {
                if kind.is_none_or(|k| t.kind.as_deref() == Some(k.as_str())) {
                    writeln!(
                        out,
                        "{}\t{}\t{}",
                        t.term,
                        t.kind.as_deref().unwrap_or("-"),
                        src
                    )
                    .map_err(|e| io_err(&e))?;
                }
            }
            Ok(0)
        }
        "search" => {
            let q = m
                .get_one::<String>("query")
                .expect("required")
                .to_lowercase();
            let mut rows: Vec<(TermEntry, &str)> = load_project(&paths)?
                .into_iter()
                .map(|t| (t, "project"))
                .collect();
            rows.extend(
                TermsFile::builtin()
                    .terms
                    .into_iter()
                    .map(|t| (t, "builtin")),
            );
            let mut hits = 0;
            for (t, src) in rows {
                let hit = std::iter::once(&t.term)
                    .chain(&t.variants)
                    .any(|s| s.to_lowercase().contains(&q));
                if hit {
                    hits += 1;
                    writeln!(
                        out,
                        "{}\t{}\t{}",
                        t.term,
                        t.kind.as_deref().unwrap_or("-"),
                        src
                    )
                    .map_err(|e| io_err(&e))?;
                }
            }
            Ok(i32::from(hits == 0))
        }
        "add" => {
            let term = m.get_one::<String>("term").expect("required").clone();
            let ignore_case = if m.get_flag("ignore-case") {
                true
            } else if m.get_flag("case-sensitive") {
                false
            } else {
                infer_ignore_case(&term)
            };
            let entry = TermEntry {
                term: term.clone(),
                variants: m
                    .get_many::<String>("variant")
                    .map(|v| v.cloned().collect())
                    .unwrap_or_default(),
                kind: m.get_one::<String>("kind").cloned(),
                ignore_case,
                case_sensitive: false,
            };
            let file = &paths[0];
            let mut doc = TermsDoc::open(file)?;
            doc.add(&entry)?;
            doc.save(file)?;
            let how = if ignore_case {
                "ignoring case"
            } else {
                "case-sensitive"
            };
            writeln!(out, "added {term} ({how})").map_err(|e| io_err(&e))?;
            Ok(0)
        }
        "remove" => {
            let term = m.get_one::<String>("term").expect("required");
            let file = &paths[0];
            let mut doc = TermsDoc::open(file)?;
            if doc.remove(term) {
                doc.save(file)?;
                writeln!(out, "removed {term}").map_err(|e| io_err(&e))?;
                Ok(0)
            } else {
                writeln!(out, "no term {term} in {}", file.display()).map_err(|e| io_err(&e))?;
                Ok(1)
            }
        }
        "check" => {
            let mut entries = TermsFile::builtin().terms;
            let project = load_project(&paths)?;
            entries.retain(|b| !project.iter().any(|p| p.term == b.term));
            entries.extend(project);
            let set = TermSet::new(entries);
            let mut found = 0;
            for p in m.get_many::<String>("paths").expect("required") {
                let path = cwd.join(p);
                let text = std::fs::read_to_string(&path).map_err(|e| TermsError::Io {
                    path: p.clone(),
                    message: e.to_string(),
                })?;
                for f in set.check_casing(&text, &markdown_code_ranges(&text)) {
                    let (line, col) = line_col(&text, f.span.start);
                    let mut msg = String::new();
                    write!(msg, "{p}:{line}:{col}: {} -> {}", f.found, f.expected)
                        .expect("write to String");
                    writeln!(out, "{msg}").map_err(|e| io_err(&e))?;
                    found += 1;
                }
            }
            Ok(i32::from(found > 0))
        }
        "import" => {
            let from = m.get_one::<String>("from").expect("required");
            let source = Source::parse(from).expect("clap value_parser restricts values");
            let file = m.get_one::<String>("file").expect("required");
            let text = std::fs::read_to_string(cwd.join(file)).map_err(|e| TermsError::Io {
                path: file.clone(),
                message: e.to_string(),
            })?;
            let imported = import::parse(file, source, &text)?;
            let target = &paths[0];
            let mut doc = TermsDoc::open(target)?;
            let (mut added, mut existing) = (0, 0);
            for w in &imported.words {
                let entry = TermEntry {
                    term: w.clone(),
                    variants: vec![],
                    kind: None,
                    ignore_case: infer_ignore_case(w),
                    case_sensitive: false,
                };
                if doc.contains(w) {
                    existing += 1;
                } else {
                    doc.add(&entry)?;
                    added += 1;
                }
            }
            doc.save(target)?;
            writeln!(
                out,
                "imported {added} terms ({existing} already present, {} patterns skipped)",
                imported.skipped.len()
            )
            .map_err(|e| io_err(&e))?;
            for s in &imported.skipped {
                writeln!(out, "skipped pattern: {s}").map_err(|e| io_err(&e))?;
            }
            Ok(0)
        }
        _ => Ok(2),
    }
}
