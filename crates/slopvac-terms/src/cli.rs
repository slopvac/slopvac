//! The `slopvac terms` command: list, search, add, remove, check and import.
//!
//! `command()` returns the clap tree and `run()` executes a parsed match. The
//! CLI crate mounts them; nothing here owns `main`.

use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::edit::{TermsDoc, infer_ignore_case};
use crate::import::{self, Source};
use crate::messages::{TermsPhrase, TermsText};
use crate::{EntryProblem, TermEntry, TermSet, TermsError, TermsFile};

const DEFAULT_FILE: &str = ".slopvac/terms.toml";

fn files_arg() -> Arg {
    Arg::new("files")
        .long("files")
        .value_name("PATH")
        .action(ArgAction::Append)
        .help(TermsPhrase::FilesHelp.as_static())
}

/// The `terms` command tree.
#[must_use]
pub fn command() -> Command {
    Command::new("terms")
        .about(TermsPhrase::About.as_static())
        .subcommand_required(true)
        .subcommand(
            Command::new("list")
                .about(TermsPhrase::ListAbout.as_static())
                .arg(Arg::new("kind").long("kind").value_name("KIND"))
                .arg(
                    Arg::new("builtin")
                        .long("builtin")
                        .action(ArgAction::SetTrue)
                        .help(TermsPhrase::BuiltinHelp.as_static()),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("search")
                .about(TermsPhrase::SearchAbout.as_static())
                .arg(Arg::new("query").required(true))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("add")
                .about(TermsPhrase::AddAbout.as_static())
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
                        .action(ArgAction::SetTrue)
                        .help(TermsPhrase::CaseSensitiveHelp.as_static()),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("remove")
                .about(TermsPhrase::RemoveAbout.as_static())
                .arg(Arg::new("term").required(true))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("check")
                .about(TermsPhrase::CheckAbout.as_static())
                .arg(Arg::new("paths").required(true).num_args(1..))
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("suggest")
                .about(TermsPhrase::SuggestAbout.as_static())
                .arg(Arg::new("paths").required(true).num_args(1..))
                .arg(
                    Arg::new("min")
                        .long("min")
                        .value_name("N")
                        .default_value("2")
                        .value_parser(clap::value_parser!(usize))
                        .help(TermsPhrase::MinHelp.as_static()),
                )
                .arg(
                    Arg::new("accept")
                        .long("accept")
                        .value_name("TERM")
                        .action(ArgAction::Append)
                        .help(TermsPhrase::AcceptHelp.as_static()),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("import")
                .about(TermsPhrase::ImportAbout.as_static())
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

/// The name of `path` in messages: relative to `cwd`, the project root, or `<external>/<name>`.
fn label(cwd: &Path, path: &Path) -> String {
    slopvac_fs::display_under_root(path, cwd)
}

/// Read a file named on the command line, labelled relative to `cwd` in errors.
fn read_input(cwd: &Path, given: &str, cap: u64) -> Result<String, TermsError> {
    let path = cwd.join(given);
    crate::read::read_existing(&path, &label(cwd, &path), cap)
}

fn load_project(paths: &[PathBuf], cwd: &Path) -> Result<Vec<TermEntry>, TermsError> {
    let mut out = Vec::new();
    for p in paths {
        if p.exists() {
            out.extend(TermsFile::load_labeled(p, &label(cwd, p))?.terms);
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
        inline_spans(line, base, &mut out);
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Inline code spans in one line. A span opens with a run of backticks and
/// closes at the next run of the same length, so ``` `` a`b `` ``` is one span.
fn inline_spans(line: &str, base: usize, out: &mut Vec<Range<usize>>) {
    let runs: Vec<(usize, usize)> = {
        let mut runs = Vec::new();
        let mut start = None;
        for (i, c) in line.char_indices() {
            match (c == '`', start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    runs.push((s, i - s));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, line.len() - s));
        }
        runs
    };
    let mut k = 0;
    while k < runs.len() {
        let (s, n) = runs[k];
        if let Some(j) = (k + 1..runs.len()).find(|&j| runs[j].1 == n) {
            out.push(base + s..base + runs[j].0 + n);
            k = j + 1;
        } else {
            k += 1;
        }
    }
}

fn rows(project: &[TermEntry], builtin: bool) -> Vec<(TermEntry, &'static str)> {
    let mut rows: Vec<(TermEntry, &'static str)> =
        project.iter().cloned().map(|t| (t, "project")).collect();
    if builtin {
        rows.extend(
            TermsFile::builtin()
                .terms
                .into_iter()
                .map(|t| (t, "builtin")),
        );
    }
    rows
}

fn print_row(out: &mut dyn Write, t: &TermEntry, src: &str) -> Result<(), TermsError> {
    writeln!(
        out,
        "{}\t{}\t{}",
        t.term,
        t.kind.as_deref().unwrap_or("-"),
        src
    )
    .map_err(|e| io_err(&e))
}

fn cmd_list(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let kind = m.get_one::<String>("kind");
    for (t, src) in rows(&load_project(paths, cwd)?, m.get_flag("builtin")) {
        if kind.is_none_or(|k| t.kind.as_deref() == Some(k.as_str())) {
            print_row(out, &t, src)?;
        }
    }
    Ok(0)
}

fn cmd_search(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let q = m
        .get_one::<String>("query")
        .expect("required")
        .to_lowercase();
    let mut hits = 0;
    for (t, src) in rows(&load_project(paths, cwd)?, true) {
        let hit = std::iter::once(&t.term)
            .chain(&t.variants)
            .any(|s| s.to_lowercase().contains(&q));
        if hit {
            hits += 1;
            print_row(out, &t, src)?;
        }
    }
    Ok(i32::from(hits == 0))
}

fn cmd_add(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let term = m.get_one::<String>("term").expect("required").clone();
    let case_sensitive = m.get_flag("case-sensitive");
    let ignore_case = if m.get_flag("ignore-case") {
        true
    } else if case_sensitive {
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
        case_sensitive,
    };
    let file = &paths[0];
    let mut doc = TermsDoc::open_labeled(file, &label(cwd, file))?;
    doc.add(&entry)?;
    doc.save_labeled(file, &label(cwd, file))?;
    let mode = if ignore_case {
        TermsPhrase::ModeIgnoreCase
    } else if case_sensitive {
        TermsPhrase::ModeExact
    } else {
        TermsPhrase::ModeFlagsWrongCase
    };
    let added = TermsText::AddedWithMode {
        term,
        mode: mode.as_static().to_owned(),
    };
    writeln!(out, "{added}").map_err(|e| io_err(&e))?;
    Ok(0)
}

fn cmd_remove(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let term = m.get_one::<String>("term").expect("required");
    let file = &paths[0];
    let mut doc = TermsDoc::open_labeled(file, &label(cwd, file))?;
    if doc.remove(term) {
        doc.save_labeled(file, &label(cwd, file))?;
        let removed = TermsText::Removed { term: term.clone() };
        writeln!(out, "{removed}").map_err(|e| io_err(&e))?;
        Ok(0)
    } else {
        let missing = TermsText::RemoveMissing {
            term: term.clone(),
            file: label(cwd, file),
        };
        writeln!(out, "{missing}").map_err(|e| io_err(&e))?;
        Ok(1)
    }
}

fn cmd_check(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let mut entries = TermsFile::builtin().terms;
    let project = load_project(paths, cwd)?;
    entries.retain(|b| !project.iter().any(|p| p.term == b.term));
    entries.extend(project);
    let set = TermSet::new(entries);
    let mut found = 0;
    for p in m.get_many::<String>("paths").expect("required") {
        let text = read_input(cwd, p, slopvac_fs::MAX_DOCUMENT_BYTES)?;
        for f in set.check_casing(&text, &markdown_code_ranges(&text)) {
            let (line, col) = line_col(&text, f.span.start);
            let finding = TermsText::CheckFinding {
                file: p.clone(),
                line,
                column: col,
                found: f.found,
                expected: f.expected,
            };
            writeln!(out, "{finding}").map_err(|e| io_err(&e))?;
            found += 1;
        }
    }
    Ok(i32::from(found > 0))
}

fn cmd_suggest(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let project = load_project(paths, cwd)?;
    let known: std::collections::BTreeSet<String> = TermsFile::builtin()
        .terms
        .iter()
        .chain(&project)
        .flat_map(|t| std::iter::once(&t.term).chain(&t.variants))
        .map(|s| s.to_lowercase())
        .collect();
    let mut texts = Vec::new();
    for p in m.get_many::<String>("paths").expect("required") {
        let text = read_input(cwd, p, slopvac_fs::MAX_DOCUMENT_BYTES)?;
        texts.push((p.clone(), text));
    }
    let inputs: Vec<_> = texts
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str(), markdown_code_ranges(t)))
        .collect();
    let min = *m.get_one::<usize>("min").expect("has a default");
    let found = crate::suggest::suggest(&inputs, &known, min);
    let accept: Vec<&String> = m
        .get_many::<String>("accept")
        .map(Iterator::collect)
        .unwrap_or_default();
    if accept.is_empty() {
        for s in &found {
            writeln!(out, "{}\t{}\t{}\t{}", s.term, s.count, s.files, s.first)
                .map_err(|e| io_err(&e))?;
        }
        return Ok(0);
    }
    if let Some(bad) = accept
        .iter()
        .find(|a| !found.iter().any(|s| &s.term == **a))
    {
        return Err(TermsError::Entry {
            path: "suggest".to_owned(),
            message: TermsText::AcceptUnknown {
                term: (*bad).clone(),
            }
            .to_string(),
            number: None,
            problem: EntryProblem::Other,
        });
    }
    let target = &paths[0];
    let mut doc = TermsDoc::open_labeled(target, &label(cwd, target))?;
    for term in &accept {
        doc.add(&TermEntry {
            term: (*term).clone(),
            variants: vec![],
            kind: None,
            ignore_case: infer_ignore_case(term),
            case_sensitive: false,
        })?;
    }
    doc.save_labeled(target, &label(cwd, target))?;
    for term in accept {
        writeln!(out, "{}", TermsText::Added { term: term.clone() }).map_err(|e| io_err(&e))?;
    }
    Ok(0)
}

fn cmd_import(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let from = m.get_one::<String>("from").expect("required");
    let source = Source::parse(from).expect("clap value_parser restricts values");
    let file = m.get_one::<String>("file").expect("required");
    let text = read_input(cwd, file, crate::MAX_TERMS_FILE_BYTES)?;
    let imported = import::parse(&label(cwd, &cwd.join(file)), source, &text)?;
    let target = &paths[0];
    let mut doc = TermsDoc::open_labeled(target, &label(cwd, target))?;
    let (mut added, mut existing) = (0, 0);
    for w in &imported.words {
        if doc.contains(w) {
            existing += 1;
            continue;
        }
        doc.add(&TermEntry {
            term: w.clone(),
            variants: vec![],
            kind: None,
            ignore_case: infer_ignore_case(w),
            case_sensitive: false,
        })?;
        added += 1;
    }
    doc.save_labeled(target, &label(cwd, target))?;
    let summary = TermsText::ImportSummary {
        added,
        existing,
        skipped: imported.skipped.len(),
    };
    writeln!(out, "{summary}").map_err(|e| io_err(&e))?;
    for s in &imported.skipped {
        let skipped = TermsText::ImportSkipped { pattern: s.clone() };
        writeln!(out, "{skipped}").map_err(|e| io_err(&e))?;
    }
    Ok(0)
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
        "list" => cmd_list(m, cwd, &paths, out),
        "search" => cmd_search(m, cwd, &paths, out),
        "add" => cmd_add(m, cwd, &paths, out),
        "remove" => cmd_remove(m, cwd, &paths, out),
        "check" => cmd_check(m, cwd, &paths, out),
        "import" => cmd_import(m, cwd, &paths, out),
        "suggest" => cmd_suggest(m, cwd, &paths, out),
        _ => Ok(2),
    }
}
