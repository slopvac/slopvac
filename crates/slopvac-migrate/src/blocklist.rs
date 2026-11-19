//! Rewrite of the 2.x word blocklists that a config's `vocabulary.path` names.
//!
//! A 3.0 blocklist needs `schema_version = 3` and the `pos` spellings the schema lists.
//! 2.x had neither rule, so a migrated config would point at a file the 3.0 loader refuses.
//! This module plans one more change per referenced file and checks the result with the
//! real `slopvac-terms` loader before the plan is returned.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use slopvac_terms::{Blocklist, Pos, TermsError};
use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::config::{key_lines, line_of};
use crate::limits::{Budget, Kind, Refused};
use crate::{Dropped, Error};

/// A `vocabulary.path` in a 2.x config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reference {
    /// The key, such as `vocabulary.path` or `overrides[0].vocabulary.path`.
    pub(crate) key: String,
    /// The path as written, relative to the config's directory.
    pub(crate) path: String,
}

/// The blocklist paths a 2.x config names, at the project level and in each override.
pub(crate) fn references(config: &str) -> Vec<Reference> {
    let Ok(doc) = config.parse::<DocumentMut>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect(doc.as_table(), "", &mut out);
    if let Some(overrides) = doc.get("overrides") {
        let tables: Vec<(usize, &dyn TableLike)> = match overrides {
            Item::ArrayOfTables(aot) => aot
                .iter()
                .enumerate()
                .map(|(i, t)| (i, t as &dyn TableLike))
                .collect(),
            Item::Value(Value::Array(arr)) => arr
                .iter()
                .enumerate()
                .filter_map(|(i, v)| Some((i, v.as_inline_table()? as &dyn TableLike)))
                .collect(),
            _ => Vec::new(),
        };
        for (i, table) in tables {
            collect(table, &format!("overrides[{i}]."), &mut out);
        }
    }
    out
}

fn collect(table: &dyn TableLike, prefix: &str, out: &mut Vec<Reference>) {
    let path = table
        .get("vocabulary")
        .and_then(Item::as_table_like)
        .and_then(|v| v.get("path"))
        .and_then(Item::as_str);
    if let Some(path) = path {
        out.push(Reference {
            key: format!("{prefix}vocabulary.path"),
            path: path.to_owned(),
        });
    }
}

/// What planning one referenced blocklist decided.
#[derive(Debug)]
pub(crate) enum Planned {
    /// Nothing to do: already planned through another reference, or it loads as it is.
    Skip,
    /// Replace `old` with `new`.
    Rewrite {
        /// The path relative to the project root.
        path: PathBuf,
        old: String,
        new: String,
        notes: Vec<String>,
    },
    /// The blocklist cannot be migrated. Each record says why.
    Blocked(Vec<Dropped>),
}

/// Where a reference points, relative to the project root.
enum Resolved {
    /// The file or directory the path names. No component on the way is a symlink.
    Found(PathBuf),
    /// Something on the way does not exist, or a file stands where a directory must be. The
    /// path is the reference below the project root, for a report.
    Missing(PathBuf),
}

/// Follow a reference one component at a time, as the runtime does, and refuse any symlink.
///
/// `..` is applied to the real directories walked so far. Because none of them is a symlink,
/// the result is the file the runtime loads for the unchanged reference. The lexical result
/// can differ from it only through a link, which this refuses.
///
/// # Errors
/// [`Error::Unsafe`] when the path leaves the project root or a component is a symlink.
fn resolve(
    root: &Path,
    base: &Dir,
    dir: &Path,
    reference: &Reference,
    site: &(&str, usize),
) -> Result<Resolved, Error> {
    let outside = || {
        Error::Unsafe(format!(
            "{}:{}: `{}`: the blocklist is outside the project root, so migrate does not rewrite it. \
             Move it into the project, or give it `schema_version = 3` by hand",
            site.0, site.1, reference.key
        ))
    };
    let given = Path::new(&reference.path);
    let rest: PathBuf = if given.is_absolute() {
        // The root may be relative (the default is `.`), so compare with its absolute forms.
        let absolute = std::path::absolute(root).map_err(|e| Error::io(root, e))?;
        let canonical = root.canonicalize().ok();
        [Some(absolute), canonical]
            .into_iter()
            .flatten()
            .find_map(|r| given.strip_prefix(r).ok().map(Path::to_path_buf))
            .ok_or_else(outside)?
    } else {
        dir.strip_prefix(root).unwrap_or(Path::new("")).join(given)
    };
    // Each entry is a component on the way, with whether it is a real directory. A later
    // component needs one below it, and so does a `..` that cancels it.
    let mut stack: Vec<(OsString, bool)> = Vec::new();
    for component in rest.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match stack.pop() {
                None => return Err(outside()),
                Some((_, true)) => {}
                Some((_, false)) => return Ok(Resolved::Missing(rest)),
            },
            Component::Normal(name) => {
                if stack.last().is_some_and(|(_, is_dir)| !is_dir) {
                    return Ok(Resolved::Missing(rest));
                }
                stack.push((name.to_owned(), false));
                let so_far: PathBuf = stack.iter().map(|(n, _)| n).collect();
                match base.symlink_metadata(&so_far) {
                    Ok(meta) if meta.is_symlink() => {
                        return Err(Error::Unsafe(format!(
                            "{}:{}: `{}`: the blocklist path crosses a symbolic link, so migrate cannot tell which file the project loads. \
                             Name the file without the link",
                            site.0, site.1, reference.key
                        )));
                    }
                    Ok(meta) => {
                        if let Some(top) = stack.last_mut() {
                            top.1 = meta.is_dir();
                        }
                    }
                    Err(e)
                        if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
                    {
                        return Ok(Resolved::Missing(rest));
                    }
                    Err(e) => return Err(Error::io(&root.join(&so_far), e)),
                }
            }
            Component::RootDir | Component::Prefix(_) => return Err(outside()),
        }
    }
    Ok(Resolved::Found(stack.iter().map(|(n, _)| n).collect()))
}

/// Plan the rewrite of the blocklist `reference` names. The reference sits at `line` of
/// the config `config`.
///
/// `seen` holds the files already planned, so a file two settings share is planned once.
///
/// # Errors
/// [`Error::Unsafe`] for a path outside the project or through a symlink, and the read errors
/// of [`Budget::read_in`].
pub(crate) fn plan_reference(
    root: &Path,
    dir: &Path,
    reference: &Reference,
    config: &str,
    line: usize,
    seen: &mut BTreeSet<PathBuf>,
    budget: &mut Budget,
) -> Result<Planned, Error> {
    let base = Dir::open_ambient_dir(root, ambient_authority()).map_err(|e| Error::io(root, e))?;
    let blocked = |label: &str, reason: &str| {
        Ok(Planned::Blocked(vec![Dropped::new(
            config,
            reference.key.as_str(),
            line,
            format!("blocklist `{label}`: {reason}"),
        )]))
    };
    let rel = match resolve(root, &base, dir, reference, &(config, line))? {
        Resolved::Found(rel) => rel,
        Resolved::Missing(rest) => return blocked(&rest.display().to_string(), "file not found"),
    };
    if !seen.insert(rel.clone()) {
        return Ok(Planned::Skip);
    }
    let label = rel.display().to_string();
    let full = root.join(&rel);
    let old = match budget.read_in(&base, &rel, &full, Kind::Terms)? {
        Ok(text) => text,
        Err(Refused::NotFound) => return blocked(&label, "file not found"),
        Err(Refused::NotRegular) => return blocked(&label, "not a regular file"),
        Err(Refused::MultiplyLinked) => return blocked(&label, "has more than one hard link"),
    };
    let format = full
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match rewrite(&label, &format, &old) {
        Ok(None) => Ok(Planned::Skip),
        Ok(Some((new, notes))) => {
            budget.check_text(&full, Kind::Terms, &new)?;
            Ok(Planned::Rewrite {
                path: rel,
                old,
                new,
                notes,
            })
        }
        Err(dropped) => Ok(Planned::Blocked(dropped)),
    }
}

/// How 3.0 reads a `pos` value that 2.x accepted.
#[derive(Debug, PartialEq, Eq)]
enum PosFix {
    /// 3.0 loads the value as it is.
    Keep,
    /// 2.x accepted the value after trimming or lower-casing it. 3.0 wants this long form.
    To(&'static str),
    /// 2.x did not accept the value either.
    Unknown,
}

fn long_form(pos: Pos) -> &'static str {
    match pos {
        Pos::Noun => "noun",
        Pos::Verb => "verb",
        Pos::Adjective => "adjective",
        Pos::Adverb => "adverb",
        Pos::Preposition => "preposition",
        Pos::Conjunction => "conjunction",
        Pos::Pronoun => "pronoun",
        Pos::Article => "article",
    }
}

/// Python's `str.strip()`, which 2.x used. It removes what Rust's `trim` removes, and also
/// U+001C to U+001F.
fn py_strip(raw: &str) -> &str {
    raw.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// 2.x trimmed the value, tried it as it was, then lower-cased it. 3.0 matches the listed
/// spellings exactly, so the two fallbacks become the long form.
fn fix_pos(raw: &str) -> PosFix {
    if Pos::parse(raw).is_some() {
        return PosFix::Keep;
    }
    let trimmed = py_strip(raw);
    match Pos::parse(trimmed).or_else(|| Pos::parse(&trimmed.to_lowercase())) {
        Some(pos) => PosFix::To(long_form(pos)),
        None => PosFix::Unknown,
    }
}

/// The entry tables with their index in `entries`, the index `key_lines` uses.
fn entry_tables(entries: &mut Item) -> Vec<(usize, &mut dyn TableLike)> {
    match entries {
        Item::ArrayOfTables(aot) => aot
            .iter_mut()
            .enumerate()
            .map(|(i, t)| (i, t as &mut dyn TableLike))
            .collect(),
        Item::Value(Value::Array(arr)) => arr
            .iter_mut()
            .enumerate()
            .filter_map(|(i, v)| Some((i, v.as_inline_table_mut()? as &mut dyn TableLike)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Rewrite one blocklist to 3.0. `None` when the 3.0 loader already takes the text.
///
/// Only TOML is rewritten: the edit keeps its comments and layout. A JSON or YAML blocklist
/// that 3.0 refuses is reported, because a text edit of those formats could change what the
/// file means. Line numbers refer to `text`, which is what the user has on disk.
fn rewrite(
    label: &str,
    format: &str,
    text: &str,
) -> Result<Option<(String, Vec<String>)>, Vec<Dropped>> {
    let blocked =
        |line: usize, key: &str, reason: String| vec![Dropped::new(label, key, line, reason)];
    let Err(refused) = Blocklist::parse(label, format, text) else {
        return Ok(None);
    };
    if format != "toml" {
        let reason = if matches!(format, "json" | "yml" | "yaml") {
            format!(
                "3.0 refuses this .{format} blocklist ({}). migrate rewrites TOML blocklists only. \
                 Add `schema_version: 3` and long-form `pos` values by hand, or convert it to TOML",
                describe(&refused)
            )
        } else {
            "the suffix is not .toml, .json, .yml or .yaml, so 3.0 cannot load it".to_owned()
        };
        return Err(blocked(
            located(&refused, &HashMap::new()).1,
            "schema_version",
            reason,
        ));
    }
    // A parser's message can quote the file, so no message is repeated. The position comes
    // from the TOML syntax tree, not from the message text.
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        blocked(
            e.span().map_or(1, |s| line_of(text, s.start)),
            "entries",
            "the file is not valid TOML".to_owned(),
        )
    })?;
    let lines = key_lines(text).unwrap_or_default();
    if matches!(refused, TermsError::Parse { .. }) {
        let (key, line) = wrong_type_at(&doc, &lines);
        return Err(blocked(
            line,
            &key,
            format!("the file {}", describe(&refused)),
        ));
    }
    let line = |key: &str| lines.get(key).copied().unwrap_or(1);
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    match doc.get("schema_version") {
        None => {
            doc.insert("schema_version", toml_edit::value(3));
            notes.push(format!("{label}: added schema_version = 3"));
        }
        Some(item) if item.as_integer() == Some(3) => {}
        Some(_) => problems.extend(blocked(
            line("schema_version"),
            "schema_version",
            "a 2.x blocklist has no `schema_version`; this value is not 3, so migrate leaves it for you to fix"
                .to_owned(),
        )),
    }
    if let Some(entries) = doc.get_mut("entries") {
        for (index, table) in entry_tables(entries) {
            let Some(raw) = table.get("pos").and_then(Item::as_str).map(str::to_owned) else {
                continue;
            };
            let key = format!("entries[{index}]");
            let pos_line = lines
                .get(&format!("{key}.pos"))
                .copied()
                .unwrap_or_else(|| line(&key));
            match fix_pos(&raw) {
                PosFix::Keep => {}
                PosFix::To(long) => {
                    if let Some(item) = table.get_mut("pos") {
                        let mut value = Value::from(long);
                        if let Some(old) = item.as_value() {
                            *value.decor_mut() = old.decor().clone();
                        }
                        *item = Item::Value(value);
                    }
                    notes.push(format!("{label}:{pos_line}: pos rewritten to `{long}`"));
                }
                PosFix::Unknown => problems.extend(blocked(
                    pos_line,
                    &format!("{key}.pos"),
                    "unrecognized part of speech; use noun, verb, adjective, adverb, \
                     preposition, conjunction, pronoun or article"
                        .to_owned(),
                )),
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let new = doc.to_string();
    // The real loader has the last word, so a file migrate writes always loads.
    match Blocklist::parse(label, "toml", &new) {
        Ok(_) => Ok(Some((new, notes))),
        Err(e) => Err(blocked_by(label, &e, &lines)),
    }
}

/// The key and one-based line a loader failure points at, in the file on disk. An entry
/// error names the entry (the loader numbers entries from one). A parse error carries its
/// position in the parser's message, and that is all this reads from the message.
fn located(error: &TermsError, lines: &HashMap<String, usize>) -> (String, usize) {
    match error {
        TermsError::Entry {
            number: Some(n), ..
        } => {
            let key = format!("entries[{}]", n.saturating_sub(1));
            let line = lines.get(&key).copied().unwrap_or(1);
            (key, line)
        }
        // The error holds a message and no position. The message can quote the file, so it is
        // never searched for one.
        TermsError::Parse { .. } => ("entries".to_owned(), 1),
        _ => (
            "entries".to_owned(),
            lines.get("entries").copied().unwrap_or(1),
        ),
    }
}

/// The key and line of the first entry field that is not a string, in a TOML blocklist the
/// loader refused for its types. Falls back to the `entries` key.
fn wrong_type_at(doc: &DocumentMut, lines: &HashMap<String, usize>) -> (String, usize) {
    if let Some(Item::ArrayOfTables(entries)) = doc.get("entries") {
        for (index, table) in entries.iter().enumerate() {
            for field in ["word", "pos", "reason", "replacement"] {
                if table.get(field).is_some_and(|item| item.as_str().is_none()) {
                    let key = format!("entries[{index}].{field}");
                    let line = lines.get(&key).copied().unwrap_or(1);
                    return (key, line);
                }
            }
        }
    }
    (
        "entries".to_owned(),
        lines.get("entries").copied().unwrap_or(1),
    )
}

/// A loader failure as a located record.
fn blocked_by(label: &str, error: &TermsError, lines: &HashMap<String, usize>) -> Vec<Dropped> {
    let (key, line) = located(error, lines);
    vec![Dropped::new(
        label,
        key,
        line,
        format!("3.0 refuses the migrated blocklist: {}", describe(error)),
    )]
}

/// A content-free category for a loader failure. The loader's own messages can quote the file
/// (a parser echoes the text it stopped at, an unknown key is a word from the file), and
/// policies section 1.7 keeps file content out of messages.
fn describe(error: &TermsError) -> String {
    match error {
        TermsError::Entry {
            problem,
            number: Some(n),
            ..
        } => format!("entry {n} {}", problem.describe()),
        TermsError::Entry { problem, .. } => format!("an entry {}", problem.describe()),
        TermsError::MissingSchemaVersion { .. } => "has no `schema_version`".to_owned(),
        TermsError::SchemaVersion { .. } => "has an unsupported `schema_version`".to_owned(),
        TermsError::UnknownKey { .. } => "has an unknown key".to_owned(),
        TermsError::Parse { .. } => "is not valid for its format".to_owned(),
        _ => "cannot be read".to_owned(),
    }
}
