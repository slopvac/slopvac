//! Planning and previewing a migration.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use similar::TextDiff;
use slopvac_engine::discover::FileKind;
use toml_edit::{DocumentMut, Item, Value};

use crate::blocklist::{self, Planned};
use crate::capped::{Blockers, Capped, Listed};
use crate::config::migrate_config_in;
use crate::limits::{Budget, Kind, Limits};
use crate::messages::{MigratePhrase, MigrateText};
use crate::nesting::Refusal;
use crate::provenance::{KeyLines, key_lines};
use crate::walk::{Dir, discover};
use crate::{Dropped, Error, Options, RenameTable, rewrite_directives, rewrite_steering};

/// Where `apply` keeps the files it replaced, relative to the project root.
pub const BACKUP_DIR: &str = ".slopvac-migrate-backup";

/// What a change does to its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Create or overwrite the file.
    Write,
    /// Delete the file.
    Delete,
}

/// One file edit, with the path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The file.
    pub path: PathBuf,
    /// Write or delete.
    pub kind: ChangeKind,
    /// The text on disk now, if the file exists.
    pub old: Option<String>,
    /// The text to write. `None` for a delete.
    pub new: Option<String>,
}

/// Every edit a migration makes, plus what it found worth saying.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// File edits in the order `apply` performs them.
    pub changes: Vec<Change>,
    /// Config changes that are more than a plain copy.
    pub notes: Vec<String>,
    /// Things the user should check, such as a malformed steering block.
    pub warnings: Vec<String>,
}

impl Plan {
    /// True when nothing needs to change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The new text for each written file, for linting the migrated tree.
    #[must_use]
    pub fn overlay(&self, root: &Path) -> Vec<(PathBuf, String)> {
        self.changes
            .iter()
            .filter_map(|c| Some((root.join(&c.path), c.new.clone()?)))
            .collect()
    }
}

/// What planning collects: the edits, and the bounded notes and warnings.
struct Sink {
    changes: Vec<Change>,
    notes: Capped,
    warnings: Capped,
    /// Blocklists already planned, so a file two settings share is planned once.
    blocklists: BTreeSet<PathBuf>,
}

/// Plan the migration of the project under `root`.
///
/// `steering` is the 3.0 managed block, as `slopvac setup` writes it. Reads only. Every unrepresentable setting across all files comes back together in
/// [`Error::Dropped`].
///
/// # Errors
/// A setting 3.0 cannot represent, or a file that is unreadable, not UTF-8, over its size
/// cap, a malformed `pyproject.toml`, or over the default per-run [`Limits`].
pub fn plan(
    root: &Path,
    renames: &RenameTable,
    options: Options,
    steering: &str,
) -> Result<Plan, Error> {
    plan_with_limits(root, renames, options, steering, Limits::default())
}

/// Like [`plan`], with explicit per-run `limits`.
///
/// # Errors
/// As [`plan`]; reading more files or bytes than `limits` allow is [`Error::RunLimit`].
pub fn plan_with_limits(
    root: &Path,
    renames: &RenameTable,
    options: Options,
    steering: &str,
    limits: Limits,
) -> Result<Plan, Error> {
    let mut budget = Budget::new(limits);
    let mut sink = Sink {
        changes: Vec::new(),
        notes: Capped::new(Listed::Notes),
        warnings: Capped::new(Listed::Warnings),
        blocklists: BTreeSet::new(),
    };
    let mut dropped = Blockers::default();
    let tree = discover(root, &mut budget, &mut sink.warnings)?;
    for dir in &tree.dirs {
        plan_dir(
            root,
            dir,
            renames,
            options,
            &mut budget,
            &mut sink,
            &mut dropped,
        )?;
    }
    for (doc, kind) in &tree.docs {
        plan_document(
            root,
            doc,
            kind,
            renames,
            steering,
            &mut budget,
            &mut sink,
            &mut dropped,
        )?;
    }
    let plan = Plan {
        changes: sink.changes,
        notes: sink.notes.finish(),
        warnings: sink.warnings.finish(),
    };
    let dropped = dropped.finish();
    if dropped.is_empty() {
        Ok(plan)
    } else {
        Err(Error::Dropped(dropped))
    }
}

fn rel(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_owned()
}

fn plan_dir(
    root: &Path,
    found: &Dir,
    renames: &RenameTable,
    options: Options,
    budget: &mut Budget,
    plan: &mut Sink,
    dropped: &mut Blockers,
) -> Result<(), Error> {
    let dir = &found.path;
    let main_cfg = dir.join("slopvac.toml");
    let dot = dir.join(".slopvac.toml");
    let pyproject = dir.join("pyproject.toml");
    let py_text = if found.pyproject {
        Some(budget.read(&pyproject, Kind::Config)?)
    } else {
        None
    };
    let py_table = match py_text.as_deref() {
        Some(text) => extract_tool_table(text, &rel(root, &pyproject).display().to_string())?,
        None => None,
    };
    let sources =
        usize::from(found.main) + usize::from(found.dot) + usize::from(py_table.is_some());
    if sources > 1 {
        return Err(Error::Conflict(
            MigrateText::ConfigSources {
                directory: rel(root, dir).display().to_string(),
            }
            .to_string(),
        ));
    }
    let (source, origin, old) = if found.main {
        (
            main_cfg.clone(),
            Origin::InPlace,
            budget.read(&main_cfg, Kind::Config)?,
        )
    } else if found.dot {
        (
            dot.clone(),
            Origin::Dotfile,
            budget.read(&dot, Kind::Config)?,
        )
    } else if let (Some((extracted, _)), true) = (&py_table, py_text.is_some()) {
        (pyproject.clone(), Origin::Pyproject, extracted.clone())
    } else {
        return Ok(());
    };
    let label = rel(root, &source).display().to_string();
    let py = py_text.as_deref().filter(|_| origin == Origin::Pyproject);
    let outer = py.and_then(|t| key_lines(t).ok());
    let rewrite = match migrate_config_in(&old, &label, renames, options, outer.as_ref()) {
        Ok(rw) => rw,
        Err(Error::AlreadyMigrated { .. }) => return Ok(()),
        Err(e) => {
            return match pyproject_lines(e, py) {
                Error::Dropped(d) => {
                    dropped.extend(d);
                    Ok(())
                }
                e => Err(e),
            };
        }
    };
    for n in rewrite.notes {
        plan.notes.push(format!("{label}: {n}"));
    }
    for d in rewrite.removed_categories {
        plan.notes
            .push(format!("{label}: {}", d.category_removed_notice()));
    }
    for d in rewrite.removed_rules {
        plan.notes
            .push(format!("{label}: {}", d.rule_removed_notice()));
    }
    let references = blocklist::references(&old);
    let lines = key_lines(&old).unwrap_or_default();
    let moved = Moved {
        root,
        label: &label,
        origin,
        old,
        new: rewrite.text,
        dot: &dot,
        pyproject: &pyproject,
        py_text,
        py_table,
    };
    moved.record(&main_cfg, plan);
    plan_blocklists(
        root,
        dir,
        &Referrer {
            label: &label,
            references: &references,
            lines: &lines,
            outer: outer.as_ref(),
        },
        budget,
        plan,
        dropped,
    )?;
    Ok(())
}

/// The blocklist settings of one config, with where to find their lines.
struct Referrer<'a> {
    label: &'a str,
    references: &'a [blocklist::Reference],
    lines: &'a KeyLines,
    outer: Option<&'a KeyLines>,
}

/// Plan the rewrite of each blocklist the config names.
fn plan_blocklists(
    root: &Path,
    dir: &Path,
    from: &Referrer<'_>,
    budget: &mut Budget,
    plan: &mut Sink,
    dropped: &mut Blockers,
) -> Result<(), Error> {
    for reference in from.references {
        let line = from
            .outer
            .and_then(|o| o.line(&format!("tool.slopvac.{}", reference.key)))
            .or_else(|| from.lines.line(&reference.key))
            .unwrap_or(1);
        let planned = blocklist::plan_reference(
            root,
            dir,
            reference,
            from.label,
            line,
            &mut plan.blocklists,
            budget,
        )?;
        match planned {
            Planned::Skip => {}
            Planned::Rewrite {
                path,
                old,
                new,
                notes,
            } => {
                for note in notes {
                    plan.notes.push(note);
                }
                plan.changes.push(Change {
                    path,
                    kind: ChangeKind::Write,
                    old: Some(old),
                    new: Some(new),
                });
            }
            Planned::Blocked(records) => dropped.extend(records),
        }
    }
    Ok(())
}

/// The 2.x config text and the 3.0 text that replaces it.
struct Moved<'a> {
    root: &'a Path,
    label: &'a str,
    origin: Origin,
    old: String,
    new: String,
    dot: &'a Path,
    pyproject: &'a Path,
    py_text: Option<String>,
    py_table: Option<(String, String)>,
}

impl Moved<'_> {
    /// Adds the changes and notes for the move to `plan`.
    fn record(self, main_cfg: &Path, plan: &mut Sink) {
        let Self {
            root,
            label,
            origin,
            old,
            new,
            dot,
            pyproject,
            py_text,
            py_table,
        } = self;
        let rewrite_text = new;
        let target = rel(root, main_cfg);
        match origin {
            Origin::InPlace => plan.changes.push(Change {
                path: target,
                kind: ChangeKind::Write,
                old: Some(old),
                new: Some(rewrite_text),
            }),
            Origin::Dotfile => {
                plan.notes.push(
                    MigrateText::NoteRenamedDotfile {
                        label: label.to_string(),
                    }
                    .to_string(),
                );
                plan.changes.push(Change {
                    path: target,
                    kind: ChangeKind::Write,
                    old: None,
                    new: Some(rewrite_text),
                });
                plan.changes.push(Change {
                    path: rel(root, dot),
                    kind: ChangeKind::Delete,
                    old: Some(old),
                    new: None,
                });
            }
            Origin::Pyproject => {
                plan.notes.push(
                    MigrateText::NoteMovedPyproject {
                        label: label.to_string(),
                    }
                    .to_string(),
                );
                plan.changes.push(Change {
                    path: target,
                    kind: ChangeKind::Write,
                    old: None,
                    new: Some(rewrite_text),
                });
                plan.changes.push(Change {
                    path: rel(root, pyproject),
                    kind: ChangeKind::Write,
                    old: py_text,
                    new: py_table.map(|(_, stripped)| stripped),
                });
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    InPlace,
    Dotfile,
    Pyproject,
}

/// Point the lines of a config error at `pyproject.toml` when the config came from
/// its `[tool.slopvac]` table. `py` is the `pyproject.toml` text in that case.
fn pyproject_lines(e: Error, py: Option<&str>) -> Error {
    match e {
        Error::OverrideOverlap {
            file,
            line,
            block,
            reason,
        } => {
            let line_of = py
                .and_then(|t| key_lines(t).ok())
                .and_then(|l| l.line(&format!("tool.slopvac.overrides[{block}]")));
            Error::OverrideOverlap {
                line: line_of.unwrap_or(line),
                file,
                block,
                reason,
            }
        }
        Error::InvalidType {
            file,
            line,
            message,
        } => {
            let line_of = py
                .and_then(|t| key_lines(t).ok())
                .and_then(|l| l.line("tool.slopvac.schema_version"));
            Error::InvalidType {
                line: line_of.unwrap_or(line),
                file,
                message,
            }
        }
        e => e,
    }
}

/// The `[tool.slopvac]` table as a standalone document, and `pyproject.toml` text
/// without it. Unrelated content and comments in `pyproject.toml` stay as they are.
/// `None` when the file has no `[tool.slopvac]` table.
///
/// # Errors
/// [`Error::Parse`] naming `file` and the line, when the text is not valid TOML.
fn extract_tool_table(text: &str, file: &str) -> Result<Option<(String, String)>, Error> {
    crate::nesting::check(text).map_err(|refusal| match refusal {
        Refusal::TooDeep(e) => Error::Dropped(vec![e.dropped(file, crate::nesting::TABLES_KEY)]),
        Refusal::Invalid(e) => Error::Parse {
            file: file.to_owned(),
            message: MigrateText::ParseAtLine {
                line: e
                    .span()
                    .map_or(1, |s| crate::config::line_of(text, s.start))
                    .to_string(),
                message: e.message().to_owned(),
            }
            .to_string(),
        },
    })?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| Error::Parse {
            file: file.to_owned(),
            message: e.to_string(),
        })?;
    Ok(split_tool_table(&mut doc))
}

fn split_tool_table(doc: &mut DocumentMut) -> Option<(String, String)> {
    let table = match doc.get("tool")?.get("slopvac")? {
        Item::Table(t) => {
            let mut t = t.clone();
            t.set_dotted(false);
            t
        }
        Item::Value(Value::InlineTable(t)) => t.clone().into_table(),
        _ => return None,
    };
    let tool = doc.get_mut("tool")?.as_table_like_mut()?;
    tool.remove("slopvac");
    if tool.is_empty() {
        doc.remove("tool");
    }
    let mut standalone = DocumentMut::new();
    *standalone.as_table_mut() = table;
    Some((standalone.to_string(), doc.to_string()))
}

/// Plan the directive and steering rewrite of one document. A directive id with no
/// rename entry stops the migration (docs/contract/config.md, section 5.2), so it
/// goes to `dropped` with the other blocking settings.
#[expect(
    clippy::too_many_arguments,
    reason = "one planning step; the arguments are its inputs"
)]
fn plan_document(
    root: &Path,
    path: &Path,
    kind: &FileKind,
    renames: &RenameTable,
    steering: &str,
    budget: &mut Budget,
    plan: &mut Sink,
    dropped: &mut Blockers,
) -> Result<(), Error> {
    let old = budget.read(path, Kind::Document)?;
    let label = rel(root, path);
    if *kind == FileKind::Mdx {
        warn_mdx_directives(&label, &old, renames, &mut plan.warnings);
        return Ok(());
    }
    let (mut text, found) = rewrite_directives(&old, renames);
    for r in &found.removed {
        plan.notes.push(
            Dropped::new(
                label.display().to_string(),
                MigrateText::DirectiveKey { id: r.id.clone() }.to_string(),
                r.line,
                MigratePhrase::ReasonRuleGone.as_static(),
            )
            .rule_removed_notice(),
        );
    }
    for line in &found.invalid {
        plan.warnings.push(
            MigrateText::DirectiveInvalid {
                path: label.display().to_string(),
                line: line.to_string(),
            }
            .to_string(),
        );
    }
    dropped.extend(found.unknown.into_iter().map(|u| {
        Dropped::new(
            label.display().to_string(),
            MigrateText::DirectiveKey { id: u.id.clone() }.to_string(),
            u.line,
            MigratePhrase::ReasonUnknownDirectiveId.as_static(),
        )
    }));
    match rewrite_steering(&text, steering) {
        Ok(Some(new)) => text = new,
        Ok(None) => {}
        Err(msg) => plan.warnings.push(format!("{}: {msg}", label.display())),
    }
    if text != old {
        plan.changes.push(Change {
            path: label,
            kind: ChangeKind::Write,
            old: Some(old),
            new: Some(text),
        });
    }
    Ok(())
}

/// MDX mixes prose with code (ESM blocks, JSX, expressions), so a text rewrite could change
/// what the page runs. Directives in MDX are left alone. Every 2.x directive the parser
/// finds is reported at the line where it starts, whether a rewrite would have changed it
/// or not.
fn warn_mdx_directives(label: &Path, text: &str, renames: &RenameTable, warnings: &mut Capped) {
    let (_, found) = rewrite_directives(text, renames);
    let mut lines: Vec<usize> = found
        .renamed
        .iter()
        .chain(&found.invalid)
        .copied()
        .chain(found.removed.iter().map(|r| r.line))
        .chain(found.unknown.iter().map(|u| u.line))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    for line in lines {
        warnings.push(
            MigrateText::DirectiveMdx {
                path: label.display().to_string(),
                line: line.to_string(),
            }
            .to_string(),
        );
    }
}

/// A unified diff of every change in the plan.
#[must_use]
pub fn render_diff(plan: &Plan) -> String {
    let mut out = String::new();
    for c in &plan.changes {
        let name = c.path.display();
        let old = c.old.as_deref().unwrap_or("");
        let new = c.new.as_deref().unwrap_or("");
        let a = if c.old.is_some() {
            format!("a/{name}")
        } else {
            "/dev/null".to_owned()
        };
        let b = if c.new.is_some() {
            format!("b/{name}")
        } else {
            "/dev/null".to_owned()
        };
        let diff = TextDiff::from_lines(old, new);
        let _ = write!(out, "{}", diff.unified_diff().header(&a, &b));
    }
    out
}
