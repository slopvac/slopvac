//! `slopvac validate`: check config, front matter and inline directives without linting.
//!
//! The CLI crate wires `command` and `run` into its tree. The checks are plain functions so the
//! language server and tests call them directly.

use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};
use ignore::WalkBuilder;

use crate::diag::{Code, SourceFile, render};
use crate::directives;
use crate::discover::Discovery;
use crate::error::ConfigError;
use crate::frontmatter::{self, Policy};
use crate::load::LoadedConfig;
use crate::presets::Presets;

/// The JSON Schema of the `slopvac:` front-matter block, for editors.
pub const FRONTMATTER_SCHEMA: &str = include_str!("../../../schemas/v3/frontmatter.schema.json");

/// The JSON Schema of `slopvac.toml`, for editors.
pub const CONFIG_SCHEMA: &str = include_str!("../../../schemas/v3/config.schema.json");

/// File extensions that `validate` reads when it walks a directory.
const DOCUMENT_EXTENSIONS: &[&str] = &["md", "markdown", "mdx", "rst", "txt", "html", "htm"];

/// One thing wrong, already rendered for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The file it was found in.
    pub file: String,
    /// The error class. `None` for an unreadable file.
    pub code: Option<Code>,
    /// Plain-text output with the source line and a pointer.
    pub rendered: String,
}

/// What a validation run found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// How many documents and config files were read.
    pub checked: usize,
    /// Every problem, in the order found.
    pub problems: Vec<Problem>,
    config_failed: bool,
}

impl Report {
    /// Process exit code: 0 when clean, 1 for problems in documents, 2 when a config could
    /// not be loaded, since then nothing else can be judged.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self.config_failed {
            2
        } else {
            u8::from(!self.problems.is_empty())
        }
    }
}

/// Validate the config that applies to `cwd` (or `explicit`) and every document under `paths`.
///
/// A path that is a directory is walked for documents. With no paths the run checks the
/// config alone.
pub fn validate(
    cwd: &Path,
    explicit: Option<&Path>,
    paths: &[PathBuf],
    presets: &dyn Presets,
) -> Report {
    let mut report = Report::default();
    let discovery = Discovery::new(presets);
    let config = match explicit {
        Some(p) => discovery.explicit(p).map(Some),
        None => discovery.nearest(cwd),
    };
    let config = match config {
        Ok(c) => c,
        Err(e) => {
            report.config_failed = true;
            report.problems.push(config_problem(&e));
            return report;
        }
    };
    if let Some(c) = &config {
        report.checked += c.files.len();
        report.problems.extend(missing_term_files(c));
    }
    let files = collect(paths, cwd);
    for file in files {
        let config = match (&config, explicit) {
            (Some(c), Some(_)) => Some(c.clone()),
            _ => match file.parent().map(|d| discovery.nearest(d)) {
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    report.config_failed = true;
                    report.problems.push(config_problem(&e));
                    continue;
                }
                None => None,
            },
        };
        check_document(&file, cwd, config.as_deref(), &mut report);
    }
    report
}

fn config_problem(e: &ConfigError) -> Problem {
    match e {
        ConfigError::Rejected(r) => Problem {
            file: r.file.clone(),
            code: r.diagnostics.first().map(|d| d.code),
            rendered: r.render(),
        },
        ConfigError::Io { path, .. } => Problem {
            file: path.clone(),
            code: None,
            rendered: e.to_string(),
        },
    }
}

fn collect(paths: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let p = if p.is_absolute() {
            p.clone()
        } else {
            cwd.join(p)
        };
        if p.is_dir() {
            let mut found: Vec<PathBuf> = WalkBuilder::new(&p)
                .standard_filters(false)
                .hidden(true)
                .build()
                .filter_map(Result::ok)
                .map(ignore::DirEntry::into_path)
                .filter(|f| f.is_file() && has_document_extension(f))
                .collect();
            found.sort();
            out.extend(found);
        } else {
            out.push(p);
        }
    }
    out
}

fn has_document_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DOCUMENT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn check_document(file: &Path, cwd: &Path, config: Option<&LoadedConfig>, report: &mut Report) {
    let shown = file.strip_prefix(cwd).unwrap_or(file).display().to_string();
    if let Some(c) = config
        && let Ok(canon) = fs::canonicalize(file)
        && let Ok(rel) = canon.strip_prefix(&c.root)
        && c.is_excluded(rel)
    {
        return;
    }
    let text = match fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            report.problems.push(Problem {
                file: shown.clone(),
                code: None,
                rendered: format!("cannot read {shown}: {e}"),
            });
            return;
        }
    };
    report.checked += 1;
    let policy = config.map_or_else(Policy::core, Policy::from_config);
    let mut diagnostics = frontmatter::read(&text, &shown, &policy).diagnostics;
    diagnostics.extend(directives::read(&text, &shown, &policy).diagnostics);
    for d in diagnostics {
        report.problems.push(Problem {
            file: shown.clone(),
            code: Some(d.code),
            rendered: render(
                SourceFile {
                    name: &shown,
                    text: &text,
                },
                std::slice::from_ref(&d),
            ),
        });
    }
}

/// `terms.files` and `terms.blocklists` entries that name no file, in the config chain.
fn missing_term_files(config: &LoadedConfig) -> Vec<Problem> {
    let mut out = Vec::new();
    for file in &config.files {
        let dir = Path::new(&file.display)
            .parent()
            .map_or_else(|| config.root.clone(), |p| config.root.join(p));
        for key in ["files", "blocklists"] {
            let Some(s) = file.settings.iter().find(|s| s.path == ["terms", key]) else {
                continue;
            };
            for name in s
                .value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
            {
                if !crate::load::normalise_join(&dir, name).is_file() {
                    let mut msg = String::new();
                    let _ = write!(
                        msg,
                        "error[config-invalid-value]: `terms.{key}` names `{name}`, which is not a file\n --> {}:{}:{}",
                        file.display,
                        s.source.line.unwrap_or(1),
                        s.source.column.unwrap_or(1),
                    );
                    out.push(Problem {
                        file: file.display.clone(),
                        code: Some(Code::InvalidValue),
                        rendered: msg,
                    });
                }
            }
        }
    }
    out
}

/// The `validate` subcommand definition.
#[must_use]
pub fn command() -> Command {
    Command::new("validate")
        .about("Check config, front matter and inline directives without linting")
        .arg(
            Arg::new("paths")
                .num_args(0..)
                .value_parser(clap::value_parser!(PathBuf))
                .help("Documents or directories to check; none checks the config alone"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Use this config for every file and skip discovery"),
        )
        .arg(
            Arg::new("schema")
                .long("schema")
                .value_parser(["frontmatter", "config"])
                .help("Print a JSON Schema for editors and exit"),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(ArgAction::SetTrue)
                .help("Print problems only"),
        )
}

/// Run the subcommand, writing to `out`. Returns the exit code.
pub fn run(
    matches: &ArgMatches,
    cwd: &Path,
    presets: &dyn Presets,
    out: &mut dyn Write,
) -> std::io::Result<u8> {
    if let Some(which) = matches.get_one::<String>("schema") {
        let schema = if which == "frontmatter" {
            FRONTMATTER_SCHEMA
        } else {
            CONFIG_SCHEMA
        };
        out.write_all(schema.as_bytes())?;
        return Ok(0);
    }
    let paths: Vec<PathBuf> = matches
        .get_many::<PathBuf>("paths")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let config = matches.get_one::<PathBuf>("config");
    let report = validate(cwd, config.map(|p| cwd.join(p)).as_deref(), &paths, presets);
    for p in &report.problems {
        writeln!(out, "{}\n", p.rendered)?;
    }
    if !matches.get_flag("quiet") {
        writeln!(
            out,
            "checked {} file(s): {} problem(s)",
            report.checked,
            report.problems.len()
        )?;
    }
    Ok(report.exit_code())
}
