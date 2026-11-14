//! The migration chain: pure steps that rewrite an old config one version at a time.
//!
//! `migrate` keeps comments (`toml_edit`). It reports every setting that has no new home in one
//! run and writes nothing until there are none. The `slopvac migrate` command (slopvac-8l4.16.1)
//! owns the preview, the file renames and the before-and-after lint.

use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::diag::{Code, Diagnostic};
use crate::error::{ConfigError, Rejected};
use crate::schema::validate_config;
use crate::tree::{Node, ParseError, Value, parse_toml};
use crate::version::{SUPPORTED, check_version};

/// Maps 2.x rule and category names to their 3.0 names.
pub trait Renames {
    /// The 3.0 id of a 2.x rule, or `None` when the rule has no 3.0 name.
    fn rule(&self, old: &str) -> Option<String>;

    /// The 3.0 name of a 2.x category, or `None` when it has none.
    fn category(&self, old: &str) -> Option<String>;
}

// STUB(slopvac-8l4.4.11): the rename table belongs to the profile and rule mapping
// (slopvac-8l4.2.10). Until it lands every name is kept as written.
/// A [`Renames`] that keeps every name.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityRenames;

impl Renames for IdentityRenames {
    fn rule(&self, old: &str) -> Option<String> {
        Some(old.to_owned())
    }

    fn category(&self, old: &str) -> Option<String> {
        Some(old.to_owned())
    }
}

/// The result of a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    /// The rewritten file.
    pub text: String,
    /// One line per change, in the order the steps made them.
    pub changes: Vec<String>,
}

struct Step {
    from: i64,
    check: fn(&Node, &dyn Renames) -> Vec<Diagnostic>,
    apply: fn(&mut DocumentMut, &dyn Renames, &mut Vec<String>),
}

const STEPS: &[Step] = &[Step {
    from: 2,
    check: check_2_to_3,
    apply: apply_2_to_3,
}];

fn reject(file: &str, text: &str, diagnostics: Vec<Diagnostic>) -> ConfigError {
    ConfigError::Rejected(Box::new(Rejected {
        file: file.to_owned(),
        text: text.to_owned(),
        diagnostics,
    }))
}

fn parse_failure(e: &ParseError, message: &str) -> Diagnostic {
    Diagnostic {
        code: Code::Parse,
        message: format!("{message}: {}", e.message),
        label: "the parser stopped here".into(),
        span: e.span.clone().unwrap_or(0..0),
        help: None,
        path: Vec::new(),
    }
}

/// Bring the config in `text` up to the version this build reads.
///
/// `file` names the file in diagnostics. A file already at the current version comes back
/// unchanged with no changes listed.
pub fn migrate(file: &str, text: &str, renames: &dyn Renames) -> Result<Migrated, ConfigError> {
    let mut current = text.to_owned();
    let mut changes = Vec::new();
    loop {
        let tree = parse_toml(&current)
            .map_err(|e| reject(file, &current, vec![parse_failure(&e, "not valid TOML")]))?;
        let version = match tree.get("schema_version").map(|n| &n.value) {
            Some(Value::Int(n)) => *n,
            None => 2,
            Some(_) => SUPPORTED,
        };
        if version == SUPPORTED {
            return Ok(Migrated {
                text: current,
                changes,
            });
        }
        let Some(step) = STEPS.iter().find(|s| s.from == version) else {
            let d = check_version(&tree, &current).err().into_iter().collect();
            return Err(reject(file, &current, d));
        };
        let dropped = (step.check)(&tree, renames);
        if !dropped.is_empty() {
            return Err(reject(file, &current, dropped));
        }
        let mut doc: DocumentMut = current.parse().map_err(|e: toml_edit::TomlError| {
            let at = e.span().unwrap_or(0..0);
            let parsed = ParseError {
                message: e.message().to_owned(),
                span: Some(at),
            };
            reject(
                file,
                &current,
                vec![parse_failure(&parsed, "not valid TOML")],
            )
        })?;
        changes.push(format!("added `schema_version = {}`", step.from + 1));
        doc.as_table_mut().remove("schema_version");
        (step.apply)(&mut doc, renames, &mut changes);
        let body = doc.to_string();
        let rewritten = format!(
            "schema_version = {}\n\n{}",
            step.from + 1,
            body.trim_start_matches('\n')
        );
        let check = parse_toml(&rewritten).map_err(|e| {
            reject(
                file,
                &rewritten,
                vec![parse_failure(&e, "the migrated file is not valid TOML")],
            )
        })?;
        if step.from + 1 == SUPPORTED {
            let problems = validate_config(&check, &rewritten);
            if !problems.is_empty() {
                return Err(reject(file, &rewritten, problems));
            }
        }
        current = rewritten;
    }
}

fn dropped(
    span: std::ops::Range<usize>,
    path: Vec<String>,
    message: String,
    help: &str,
) -> Diagnostic {
    Diagnostic {
        code: Code::LegacyDropped,
        message,
        label: "no 3.0 equivalent".into(),
        span,
        help: Some(help.to_owned()),
        path,
    }
}

/// The root and each `[[overrides]]` table, with the path of each.
fn scopes(root: &Node) -> Vec<(&Node, Vec<String>)> {
    let mut out = vec![(root, Vec::new())];
    if let Some(Node {
        value: Value::Array(items),
        ..
    }) = root.get("overrides")
    {
        for (i, item) in items.iter().enumerate() {
            out.push((item, vec!["overrides".to_owned(), i.to_string()]));
        }
    }
    out
}

fn check_2_to_3(root: &Node, renames: &dyn Renames) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (scope, prefix) in scopes(root) {
        let at = |tail: &[&str]| -> Vec<String> {
            prefix
                .iter()
                .cloned()
                .chain(tail.iter().map(|s| (*s).to_owned()))
                .collect()
        };
        if let Some(vale) = scope.get("vale") {
            check_vale(vale, &at(&["vale"]), &mut out);
        }
        if let Some(Node {
            value: Value::Table(entries),
            ..
        }) = scope.get("categories")
        {
            for e in entries {
                if renames.category(&e.key).is_none() {
                    out.push(dropped(
                        e.key_span.clone(),
                        at(&["categories", &e.key]),
                        format!("the category `{}` has no 3.0 name", e.key),
                        "remove the category or choose a 3.0 category",
                    ));
                }
            }
        }
        if let Some(Node {
            value: Value::Table(entries),
            ..
        }) = scope.get("rules")
        {
            for e in entries {
                if renames.rule(&e.key).is_none() {
                    out.push(dropped(
                        e.key_span.clone(),
                        at(&["rules", &e.key]),
                        format!("the rule `{}` has no 3.0 id", e.key),
                        "remove the entry or choose a 3.0 rule",
                    ));
                }
                if let Value::Table(fields) = &e.node.value {
                    for f in fields.iter().filter(|f| f.key != "severity") {
                        out.push(dropped(
                            f.key_span.clone(),
                            at(&["rules", &e.key, &f.key]),
                            format!(
                                "`rules.{}.{}` has no 3.0 equivalent; a rule takes a level only",
                                e.key, f.key
                            ),
                            "remove the setting",
                        ));
                    }
                }
            }
        }
    }
    out
}

fn check_vale(vale: &Node, path: &[String], out: &mut Vec<Diagnostic>) {
    let Value::Table(entries) = &vale.value else {
        return;
    };
    if matches!(
        vale.get("enabled").map(|n| &n.value),
        Some(Value::Bool(false))
    ) {
        return;
    }
    for e in entries {
        let mut p = path.to_vec();
        p.push(e.key.clone());
        out.push(dropped(
            e.key_span.clone(),
            p,
            format!(
                "`vale.{}` has no 3.0 equivalent; Vale is gone in 3.0",
                e.key
            ),
            "remove the setting, or set `enabled = false` in the `[vale]` table to drop it",
        ));
    }
}

fn apply_2_to_3(doc: &mut DocumentMut, renames: &dyn Renames, changes: &mut Vec<String>) {
    let root = doc.as_table_mut();
    migrate_scope(root, "", renames, changes);
    if let Some(Item::ArrayOfTables(overrides)) = root.get_mut("overrides") {
        for (i, table) in overrides.iter_mut().enumerate() {
            migrate_scope(table, &format!("overrides[{i}]."), renames, changes);
        }
    }
}

fn migrate_scope(t: &mut Table, prefix: &str, renames: &dyn Renames, changes: &mut Vec<String>) {
    move_locale(t, prefix, changes);
    move_vocabulary(t, prefix, changes);
    if t.remove("vale").is_some() {
        changes.push(format!("removed `{prefix}vale` (Vale is gone in 3.0)"));
    }
    rename_keys(
        t,
        "categories",
        prefix,
        &|k| renames.category(k),
        false,
        changes,
    );
    rename_keys(t, "rules", prefix, &|k| renames.rule(k), true, changes);
}

fn terms_mut(t: &mut Table) -> &mut Table {
    if !matches!(t.get("terms"), Some(Item::Table(_))) {
        t.insert("terms", Item::Table(Table::new()));
    }
    t.get_mut("terms")
        .and_then(Item::as_table_mut)
        .expect("terms was just inserted as a table")
}

fn move_locale(t: &mut Table, prefix: &str, changes: &mut Vec<String>) {
    let Some(item) = t.remove("locale") else {
        return;
    };
    let Some(loc) = item.as_table_like() else {
        t.insert("locale", item);
        return;
    };
    let default = loc.get("default").cloned();
    let allow = loc.get("allow").cloned();
    if let Some(d) = default {
        t.insert("locale", d);
        changes.push(format!(
            "moved `{prefix}locale.default` to `{prefix}locale`"
        ));
    }
    if let Some(a) = allow {
        terms_mut(t).insert("allow", a);
        changes.push(format!(
            "moved `{prefix}locale.allow` to `{prefix}terms.allow`"
        ));
    }
}

fn move_vocabulary(t: &mut Table, prefix: &str, changes: &mut Vec<String>) {
    let Some(item) = t.remove("vocabulary") else {
        return;
    };
    let path = item
        .as_table_like()
        .and_then(|v| v.get("path"))
        .and_then(Item::as_str)
        .map(str::to_owned);
    if let Some(p) = path {
        let mut list = Array::new();
        list.push(p);
        terms_mut(t).insert("blocklists", value(list));
        changes.push(format!(
            "moved `{prefix}vocabulary.path` to `{prefix}terms.blocklists`"
        ));
    }
}

fn rename_keys(
    t: &mut Table,
    key: &str,
    prefix: &str,
    rename: &dyn Fn(&str) -> Option<String>,
    bare_levels: bool,
    changes: &mut Vec<String>,
) {
    let Some(table) = t.get_mut(key).and_then(Item::as_table_like_mut) else {
        return;
    };
    let entries: Vec<(String, Item)> = table
        .iter()
        .map(|(k, v)| (k.to_owned(), v.clone()))
        .collect();
    table.clear();
    for (old, mut item) in entries {
        let new = rename(&old).unwrap_or_else(|| old.clone());
        if new != old {
            changes.push(format!(
                "renamed `{prefix}{key}.{old}` to `{prefix}{key}.{new}`"
            ));
        }
        if bare_levels {
            let level = item
                .as_table_like()
                .and_then(|r| r.get("severity"))
                .and_then(Item::as_str)
                .map(str::to_owned);
            if let Some(level) = level {
                item = value(level);
                changes.push(format!("wrote `{prefix}{key}.{new}` as a bare level"));
            }
        }
        table.insert(&new, item);
    }
}
