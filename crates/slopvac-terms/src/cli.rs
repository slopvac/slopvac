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
                        .action(ArgAction::SetTrue)
                        .help("Match only the exact spelling; other casings are different words"),
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
            Command::new("suggest")
                .about(
                    "List inferred proper nouns and acronyms that are not terms yet; \
                     nothing is written unless you name a term with --accept",
                )
                .arg(Arg::new("paths").required(true).num_args(1..))
                .arg(
                    Arg::new("min")
                        .long("min")
                        .value_name("N")
                        .default_value("2")
                        .value_parser(clap::value_parser!(usize))
                        .help("Only candidates seen at least N times"),
                )
                .arg(
                    Arg::new("accept")
                        .long("accept")
                        .value_name("TERM")
                        .action(ArgAction::Append)
                        .help("Add this suggested spelling to the terms file"),
                )
                .arg(files_arg()),
        )
        .subcommand(
            Command::new("import")
                .about(
                    "Import words from another tool: a Vale accept.txt, a cspell JSON config \
                     (YAML configs are not supported) or a codespell ignore list",
                )
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

/// The 1-based line and column of byte offset `at`, or `None` when it is not a char boundary.
fn line_col(text: &str, at: usize) -> Option<(usize, usize)> {
    let before = text.get(..at)?;
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or(before);
    Some((line, col.chars().count() + 1))
}

/// The first terms file, where `add`, `remove` and `import` write.
fn write_target(paths: &[PathBuf]) -> Result<&PathBuf, TermsError> {
    paths.first().ok_or_else(|| TermsError::Io {
        path: "terms file".to_owned(),
        message: "no terms file is configured".to_owned(),
    })
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
        let Some(&(s, n)) = runs.get(k) else {
            break;
        };
        if let Some((j, &(close, _))) = runs.iter().enumerate().skip(k + 1).find(|(_, r)| r.1 == n)
        {
            out.push(base + s..base + close + n);
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

fn cmd_list(m: &ArgMatches, paths: &[PathBuf], out: &mut dyn Write) -> Result<i32, TermsError> {
    let kind = m.get_one::<String>("kind");
    for (t, src) in rows(&load_project(paths)?, m.get_flag("builtin")) {
        if kind.is_none_or(|k| t.kind.as_deref() == Some(k.as_str())) {
            print_row(out, &t, src)?;
        }
    }
    Ok(0)
}

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_search(m: &ArgMatches, paths: &[PathBuf], out: &mut dyn Write) -> Result<i32, TermsError> {
    let q = m
        .get_one::<String>("query")
        .expect("required")
        .to_lowercase();
    let mut hits = 0;
    for (t, src) in rows(&load_project(paths)?, true) {
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

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_add(m: &ArgMatches, paths: &[PathBuf], out: &mut dyn Write) -> Result<i32, TermsError> {
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
    let file = write_target(paths)?;
    let mut doc = TermsDoc::open(file)?;
    doc.add(&entry)?;
    doc.save(file)?;
    let how = if ignore_case {
        "ignoring case"
    } else if case_sensitive {
        "exact spelling only"
    } else {
        "flags wrong case"
    };
    writeln!(out, "added {term} ({how})").map_err(|e| io_err(&e))?;
    Ok(0)
}

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_remove(m: &ArgMatches, paths: &[PathBuf], out: &mut dyn Write) -> Result<i32, TermsError> {
    let term = m.get_one::<String>("term").expect("required");
    let file = write_target(paths)?;
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

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_check(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let mut entries = TermsFile::builtin().terms;
    let project = load_project(paths)?;
    entries.retain(|b| !project.iter().any(|p| p.term == b.term));
    entries.extend(project);
    let set = TermSet::new(entries);
    let mut found = 0;
    for p in m.get_many::<String>("paths").expect("required") {
        let text = std::fs::read_to_string(cwd.join(p)).map_err(|e| TermsError::Io {
            path: p.clone(),
            message: e.to_string(),
        })?;
        for f in set.check_casing(&text, &markdown_code_ranges(&text)) {
            let at = line_col(&text, f.span.start)
                .map_or_else(|| "?:?".to_owned(), |(line, col)| format!("{line}:{col}"));
            writeln!(out, "{p}:{at}: {} -> {}", f.found, f.expected).map_err(|e| io_err(&e))?;
            found += 1;
        }
    }
    Ok(i32::from(found > 0))
}

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_suggest(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let project = load_project(paths)?;
    let known: std::collections::BTreeSet<String> = TermsFile::builtin()
        .terms
        .iter()
        .chain(&project)
        .flat_map(|t| std::iter::once(&t.term).chain(&t.variants))
        .map(|s| s.to_lowercase())
        .collect();
    let mut texts = Vec::new();
    for p in m.get_many::<String>("paths").expect("required") {
        let text = std::fs::read_to_string(cwd.join(p)).map_err(|e| TermsError::Io {
            path: p.clone(),
            message: e.to_string(),
        })?;
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
            message: format!("`{bad}` is not a suggestion; run without --accept to see them"),
        });
    }
    let target = write_target(paths)?;
    let mut doc = TermsDoc::open(target)?;
    for term in &accept {
        doc.add(&TermEntry {
            term: (*term).clone(),
            variants: vec![],
            kind: None,
            ignore_case: infer_ignore_case(term),
            case_sensitive: false,
        })?;
    }
    doc.save(target)?;
    for term in accept {
        writeln!(out, "added {term}").map_err(|e| io_err(&e))?;
    }
    Ok(0)
}

#[expect(
    clippy::expect_used,
    reason = "clap enforces the required argument or supplies its default"
)]
fn cmd_import(
    m: &ArgMatches,
    cwd: &Path,
    paths: &[PathBuf],
    out: &mut dyn Write,
) -> Result<i32, TermsError> {
    let from = m.get_one::<String>("from").expect("required");
    let source = Source::parse(from).expect("clap value_parser restricts values");
    let file = m.get_one::<String>("file").expect("required");
    let text = std::fs::read_to_string(cwd.join(file)).map_err(|e| TermsError::Io {
        path: file.clone(),
        message: e.to_string(),
    })?;
    let imported = import::parse(file, source, &text)?;
    let target = write_target(paths)?;
    let mut doc = TermsDoc::open(target)?;
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
        "list" => cmd_list(m, &paths, out),
        "search" => cmd_search(m, &paths, out),
        "add" => cmd_add(m, &paths, out),
        "remove" => cmd_remove(m, &paths, out),
        "check" => cmd_check(m, cwd, &paths, out),
        "import" => cmd_import(m, cwd, &paths, out),
        "suggest" => cmd_suggest(m, cwd, &paths, out),
        _ => Ok(2),
    }
}
