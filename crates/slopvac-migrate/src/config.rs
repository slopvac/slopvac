//! Rewrite of one 2.x config document into the 3.0 shape.
//!
//! The input is TOML text. The output keeps comments and key order. Settings with no
//! 3.0 equivalent are collected as [`Dropped`] and block the rewrite.

use std::collections::HashMap;

use toml_edit::{Array, Document, DocumentMut, Item, RawString, Table, TableLike, Value, value};

use crate::{Dropped, Error, Options, RenameTable, preserve_doc};

const THRESHOLD_KEYS: &[&str] = &[
    "max_total_per_100_words",
    "max_errors",
    "max_warnings",
    "min_score",
    "max_unicode_dashes",
];
const CATEGORY_KEYS: &[&str] = &[
    "severity",
    "minimum_severity",
    "max_per_100_words",
    "weight",
];
const GUIDANCE_KEYS: &[&str] = &["write", "review", "review_depth", "report"];
const TOP_KEYS: &[&str] = &[
    "mode",
    "profile",
    "categories",
    "rules",
    "thresholds",
    "vale",
    "locale",
    "vocabulary",
    "guidance",
    "overrides",
    "exclude",
];
const OVERRIDE_KEYS: &[&str] = &[
    "files",
    "profile",
    "categories",
    "rules",
    "thresholds",
    "vale",
    "locale",
    "vocabulary",
    "guidance",
];

/// The result of a successful rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// The 3.0 document text.
    pub text: String,
    /// One line per change that is not a plain copy.
    pub notes: Vec<String>,
    /// The categories that 3.0 no longer has and the rewrite dropped, with lines in
    /// `text`'s source. A caller that embeds the source in a larger file maps the
    /// lines first, then renders each with [`Dropped::category_removed_notice`].
    pub removed_categories: Vec<Dropped>,
}

/// Rewrite 2.x config text. `file` names the source in diagnostics.
///
/// Returns every unrepresentable setting at once, so a user fixes them in one pass.
pub fn migrate_config(
    text: &str,
    file: &str,
    renames: &RenameTable,
    options: Options,
) -> Result<Rewrite, Error> {
    let lines = key_lines(text).map_err(|e| Error::Parse {
        file: file.to_owned(),
        message: e,
    })?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| Error::Parse {
            file: file.to_owned(),
            message: e.to_string(),
        })?;
    if !is_legacy(text, file)? {
        return Err(Error::AlreadyMigrated {
            file: file.to_owned(),
        });
    }
    let original = doc.clone();
    let mut cx = Cx {
        file,
        lines: &lines,
        renames,
        dropped: Vec::new(),
        notes: Vec::new(),
        removed_categories: Vec::new(),
    };
    migrate_scope(doc.as_table_mut(), "", Scope::Project, &mut cx);
    if cx.dropped.is_empty() && options.preserve_behavior {
        preserve_doc::apply(
            &original,
            &mut doc,
            &mut preserve_doc::Ctx {
                file,
                lines: &lines,
                renames,
                dropped: &mut cx.dropped,
                notes: &mut cx.notes,
            },
        )?;
    }
    if cx.dropped.is_empty() {
        doc.insert("schema_version", value(3));
        Ok(Rewrite {
            text: doc.to_string(),
            notes: cx.notes,
            removed_categories: cx.removed_categories,
        })
    } else {
        Err(Error::Dropped(cx.dropped))
    }
}

/// Whether the migration chain starts at the 2.x to 3.0 step for this text.
///
/// The version gate of `slopvac-config` decides, so migrate and the 3.0 loader always
/// agree. A file the gate refuses as 2.x needs the step. A file it accepts, or one from a
/// newer slopvac, needs none.
fn is_legacy(text: &str, file: &str) -> Result<bool, Error> {
    let root = slopvac_config::tree::parse_toml(text).map_err(|e| Error::Parse {
        file: file.to_owned(),
        message: e.message,
    })?;
    // The version gate leaves a non-integer to the schema, so a value like `"3"` would pass
    // for migrated. Refuse it here, with its line.
    if let Some(entry) = root.entry("schema_version")
        && !matches!(entry.node.value, slopvac_config::tree::Value::Int(_))
    {
        return Err(Error::InvalidType {
            file: file.to_owned(),
            line: line_of(text, entry.node.span.start),
            message: "`schema_version` must be an integer".to_owned(),
        });
    }
    Ok(matches!(
        slopvac_config::version::check_version(&root, text),
        Err(d) if d.code == slopvac_config::Code::LegacyVersion
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Project,
    Override,
}

struct Cx<'a> {
    file: &'a str,
    lines: &'a HashMap<String, usize>,
    renames: &'a RenameTable,
    dropped: Vec<Dropped>,
    notes: Vec<String>,
    removed_categories: Vec<Dropped>,
}

impl Cx<'_> {
    /// Drop a category that 3.0 no longer has and go on, with a
    /// `migrate-category-removed` notice and no exit 2.
    fn drop_category_note(&mut self, path: &str, reason: &str) {
        self.removed_categories.push(Dropped {
            file: self.file.to_owned(),
            key: path.to_owned(),
            line: self.lines.get(path).copied().unwrap_or(1),
            reason: reason.to_owned(),
        });
    }
    fn drop_key(&mut self, path: &str, reason: impl Into<String>) {
        self.dropped.push(Dropped {
            file: self.file.to_owned(),
            key: path.to_owned(),
            line: self.lines.get(path).copied().unwrap_or(1),
            reason: reason.into(),
        });
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn keys_of(t: &dyn TableLike) -> Vec<String> {
    t.iter().map(|(k, _)| k.to_owned()).collect()
}

fn migrate_scope(t: &mut dyn TableLike, path: &str, scope: Scope, cx: &mut Cx<'_>) {
    let allowed = if scope == Scope::Project {
        TOP_KEYS
    } else {
        OVERRIDE_KEYS
    };
    for key in keys_of(t) {
        let kp = join(path, &key);
        if scope == Scope::Project && key == "schema_version" {
            if t.get("schema_version").and_then(Item::as_integer) != Some(2) {
                cx.drop_key(&kp, "expected 2 in a 2.x config");
            }
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            cx.drop_key(
                &kp,
                "no 3.0 equivalent; remove the key or rewrite it by hand",
            );
            continue;
        }
        match key.as_str() {
            "categories" => migrate_categories(t, &key, &kp, cx),
            "rules" => migrate_rules(t, &key, &kp, cx),
            "thresholds" => check_keys(t, &key, &kp, THRESHOLD_KEYS, cx),
            "locale" => migrate_locale(t, &kp, cx),
            "vocabulary" => migrate_vocabulary(t, &kp, cx),
            "vale" => migrate_vale(t, &kp, cx),
            "guidance" => {
                if scope == Scope::Override {
                    cx.drop_key(
                        &kp,
                        "2.x rejected guidance inside an override, and 3.0 does too",
                    );
                } else {
                    check_keys(t, &key, &kp, GUIDANCE_KEYS, cx);
                }
            }
            "overrides" => migrate_overrides(t, &kp, cx),
            _ => {}
        }
    }
}

/// Flag keys of a sub-table that 3.0 does not know. Known keys copy as they are.
fn check_keys(t: &mut dyn TableLike, key: &str, path: &str, known: &[&str], cx: &mut Cx<'_>) {
    let Some(sub) = t.get_mut(key).and_then(Item::as_table_like_mut) else {
        return;
    };
    for k in keys_of(sub) {
        if !known.contains(&k.as_str()) {
            cx.drop_key(&join(path, &k), "no 3.0 equivalent");
        }
    }
}

fn migrate_overrides(t: &mut dyn TableLike, path: &str, cx: &mut Cx<'_>) {
    let Some(item) = t.get_mut("overrides") else {
        return;
    };
    if let Item::ArrayOfTables(aot) = item {
        for (i, table) in aot.iter_mut().enumerate() {
            migrate_scope(table, &format!("{path}[{i}]"), Scope::Override, cx);
        }
    } else if let Some(arr) = item.as_array_mut() {
        for (i, v) in arr.iter_mut().enumerate() {
            if let Some(table) = v.as_inline_table_mut() {
                migrate_scope(table, &format!("{path}[{i}]"), Scope::Override, cx);
            }
        }
    }
}

/// Take every entry out of the map-like table at `key`, so the caller can put
/// renamed entries back in the original order with their comments.
fn drain(t: &mut dyn TableLike, key: &str) -> Option<Vec<(toml_edit::Key, Item)>> {
    let sub = t.get_mut(key)?.as_table_like_mut()?;
    let names = keys_of(sub);
    let mut out = Vec::new();
    for name in names {
        let k = sub.key(&name).cloned()?;
        let item = sub.remove(&name)?;
        out.push((k, item));
    }
    Some(out)
}

/// An entry ready to go back into its map: the new name, the old key (for its decor),
/// the new item, and comment text that sat on a table header or inner key and now
/// leads the entry.
type Entry = (String, toml_edit::Key, Item, String);

fn restore(t: &mut dyn TableLike, key: &str, entries: Vec<Entry>) {
    let Some(sub) = t.get_mut(key).and_then(Item::as_table_like_mut) else {
        return;
    };
    for (name, old, item, carried) in entries {
        let is_value = item.is_value();
        sub.insert(&name, item);
        if let Some(mut k) = sub.key_mut(&name) {
            *k.leaf_decor_mut() = old.leaf_decor().clone();
            *k.dotted_decor_mut() = old.dotted_decor().clone();
            if is_value {
                let own = k
                    .leaf_decor()
                    .prefix()
                    .and_then(RawString::as_str)
                    .unwrap_or("")
                    .to_owned();
                k.leaf_decor_mut().set_prefix(format!("{own}{carried}"));
                k.leaf_decor_mut().set_suffix(" ");
            }
        }
    }
}

/// When a table holds only `severity`, the bare level that replaces it and the
/// comments the table carried: the text before its header and before the key.
fn collapse_severity(item: &Item) -> Option<(Item, String)> {
    let tl = item.as_table_like().filter(|tl| tl.len() == 1)?;
    let sev = tl.get("severity")?.clone();
    let mut carried = String::new();
    if let Item::Table(table) = item {
        carried.push_str(&comments(
            table
                .decor()
                .prefix()
                .and_then(RawString::as_str)
                .unwrap_or(""),
        ));
        carried.push_str(&comments(
            table
                .decor()
                .suffix()
                .and_then(RawString::as_str)
                .unwrap_or(""),
        ));
    }
    if let Some(key) = tl.key("severity") {
        carried.push_str(&comments(
            key.leaf_decor()
                .prefix()
                .and_then(RawString::as_str)
                .unwrap_or(""),
        ));
    }
    Some((sev, carried))
}

fn level_only(item: Item, path: &str, allowed: &[&str], cx: &mut Cx<'_>) -> Option<(Item, String)> {
    if item.is_str() {
        return Some((item, String::new()));
    }
    let Some(tl) = item.as_table_like() else {
        cx.drop_key(path, "expected a level or a table");
        return None;
    };
    let mut ok = true;
    for (k, _) in tl.iter() {
        if !allowed.contains(&k) {
            cx.drop_key(&join(path, k), "no 3.0 equivalent");
            ok = false;
        }
    }
    if !ok {
        return None;
    }
    if let Some(collapsed) = collapse_severity(&item) {
        return Some(collapsed);
    }
    Some((item, String::new()))
}

fn migrate_categories(t: &mut dyn TableLike, key: &str, path: &str, cx: &mut Cx<'_>) {
    let Some(entries) = drain(t, key) else { return };
    let mut out = Vec::new();
    for (k, item) in entries {
        let kp = join(path, k.get());
        if cx.renames.is_removed_category(k.get()) {
            cx.drop_category_note(&kp, "category no longer exists in 3.0");
            continue;
        }
        let Some(new) = cx.renames.category(k.get()) else {
            cx.drop_key(&kp, "unknown 2.x category; the rename table has no entry");
            continue;
        };
        if let Some((item, carried)) = level_only(item, &kp, CATEGORY_KEYS, cx) {
            if new != k.get() {
                cx.notes
                    .push(format!("renamed category {} to {new}", k.get()));
            }
            out.push((new, k, item, carried));
        }
    }
    restore(t, key, out);
}

/// The note for a reference to a rule that 3.0 removed.
fn removed_rule_note(rule: &str, path: &str) -> String {
    format!("removed {path}: rule {rule} no longer exists in 3.0")
}

fn migrate_rules(t: &mut dyn TableLike, key: &str, path: &str, cx: &mut Cx<'_>) {
    let Some(entries) = drain(t, key) else { return };
    let mut out = Vec::new();
    for (k, item) in entries {
        let kp = join(path, k.get());
        if cx.renames.is_removed(k.get()) {
            cx.notes.push(removed_rule_note(k.get(), &kp));
            continue;
        }
        let Some(new) = cx.renames.rule(k.get()) else {
            cx.drop_key(&kp, "unknown 2.x rule id; the rename table has no entry");
            continue;
        };
        if item.is_table_like() {
            let Some((sev, carried)) = collapse_severity(&item) else {
                cx.drop_key(
                    &kp,
                    "a rule takes a level only; 3.0 has no other rule field",
                );
                continue;
            };
            out.push((new, k, sev, carried));
        } else if item.is_str() {
            out.push((new, k, item, String::new()));
        } else {
            cx.drop_key(&kp, "expected a level");
        }
    }
    restore(t, key, out);
}

fn terms_table(t: &mut dyn TableLike) -> Option<&mut dyn TableLike> {
    if !t.contains_key("terms") {
        t.insert("terms", Item::Table(Table::new()));
    }
    t.get_mut("terms")?.as_table_like_mut()
}

/// The comment lines in a run of decor text. Blank lines are layout, not content.
fn comments(text: &str) -> String {
    text.split_inclusive('\n')
        .map(str::trim_start)
        .filter(|l| l.starts_with('#'))
        .map(|l| {
            if l.ends_with('\n') {
                l.to_owned()
            } else {
                format!("{l}\n")
            }
        })
        .collect()
}

/// The comments before an entry: those before its key and, for a table, those before
/// its header.
fn leading(parent: &dyn TableLike, name: &str) -> String {
    let mut out = String::new();
    if let Some(Item::Table(table)) = parent.get(name) {
        out.push_str(&comments(
            table
                .decor()
                .prefix()
                .and_then(RawString::as_str)
                .unwrap_or(""),
        ));
    }
    if let Some(key) = parent.key(name) {
        out.push_str(&comments(
            key.leaf_decor()
                .prefix()
                .and_then(RawString::as_str)
                .unwrap_or(""),
        ));
    }
    out
}

fn set_leading(t: &mut dyn TableLike, name: &str, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    if let Some(mut key) = t.key_mut(name) {
        let decor = key.leaf_decor_mut();
        decor.set_prefix(prefix);
        decor.set_suffix(" ");
    }
}

fn migrate_locale(t: &mut dyn TableLike, path: &str, cx: &mut Cx<'_>) {
    let Some(item) = t.get("locale") else { return };
    let Some(tl) = item.as_table_like() else {
        return;
    };
    let mut ok = true;
    for (k, _) in tl.iter() {
        if k.starts_with("x-") {
            cx.notes
                .push(format!("dropped extension key {}", join(path, k)));
        } else if k != "default" && k != "allow" {
            cx.drop_key(&join(path, k), "no 3.0 equivalent");
            ok = false;
        }
    }
    if !ok {
        return;
    }
    let default = tl.get("default").cloned();
    let allow = tl.get("allow").cloned();
    let header = leading(t, "locale");
    let default_lead = format!("{header}{}", leading(tl, "default"));
    let allow_lead = if default.is_some() {
        leading(tl, "allow")
    } else {
        format!("{header}{}", leading(tl, "allow"))
    };
    t.remove("locale");
    if let Some(d) = default {
        t.insert("locale", d);
        set_leading(t, "locale", &default_lead);
        cx.notes.push(format!("moved {path}.default to {path}"));
    }
    if let Some(a) = allow {
        if let Some(terms) = terms_table(t) {
            terms.insert("allow", a);
            set_leading(terms, "allow", &allow_lead);
        }
        cx.notes.push(format!("moved {path}.allow to terms.allow"));
    }
}

fn migrate_vocabulary(t: &mut dyn TableLike, path: &str, cx: &mut Cx<'_>) {
    let Some(item) = t.get("vocabulary") else {
        return;
    };
    let Some(tl) = item.as_table_like() else {
        return;
    };
    let mut ok = true;
    for (k, _) in tl.iter() {
        if k.starts_with("x-") {
            cx.notes
                .push(format!("dropped extension key {}", join(path, k)));
        } else if k != "path" {
            cx.drop_key(&join(path, k), "no 3.0 equivalent");
            ok = false;
        }
    }
    if !ok {
        return;
    }
    let lead = format!("{}{}", leading(t, "vocabulary"), leading(tl, "path"));
    let file = tl.get("path").and_then(Item::as_value).cloned();
    t.remove("vocabulary");
    if let Some(mut file) = file {
        let suffix = file
            .decor()
            .suffix()
            .and_then(RawString::as_str)
            .unwrap_or("")
            .to_owned();
        file.decor_mut().clear();
        let mut arr = Array::new();
        arr.push(file);
        let mut v = Value::from(arr);
        v.decor_mut().set_prefix(" ");
        v.decor_mut().set_suffix(suffix);
        if let Some(terms) = terms_table(t) {
            terms.insert("blocklists", Item::Value(v));
            set_leading(terms, "blocklists", &lead);
        }
        cx.notes
            .push(format!("moved {path}.path to terms.blocklists"));
    }
}

fn migrate_vale(t: &mut dyn TableLike, path: &str, cx: &mut Cx<'_>) {
    let Some(tl) = t.get("vale").and_then(Item::as_table_like) else {
        return;
    };
    let enabled = tl.get("enabled").and_then(Item::as_bool);
    let mut ok = true;
    for (k, _) in tl.iter() {
        let kp = join(path, k);
        match (k, enabled) {
            ("enabled", Some(true)) => {
                cx.drop_key(
                    &kp,
                    "Vale is removed in 3.0; set enabled = false or delete the table",
                );
                ok = false;
            }
            ("enabled", None) => {
                cx.drop_key(&kp, "expected a boolean");
                ok = false;
            }
            ("enabled" | "binary" | "config" | "styles", Some(false)) => {}
            ("binary" | "config" | "styles", _) => {
                cx.drop_key(
                    &kp,
                    "Vale is removed in 3.0; set vale.enabled = false in the same table or delete it",
                );
                ok = false;
            }
            _ => {
                cx.drop_key(&kp, "no 3.0 equivalent");
                ok = false;
            }
        }
    }
    if ok && enabled == Some(false) {
        t.remove("vale");
        cx.notes
            .push(format!("deleted {path} (Vale is removed in 3.0)"));
    } else if ok {
        cx.drop_key(path, "Vale is removed in 3.0; delete the table");
    }
}

/// Map every key path in `text` to its one-based line. Array-of-tables elements use
/// `name[index]`.
pub(crate) fn key_lines(text: &str) -> Result<HashMap<String, usize>, String> {
    let doc = Document::parse(text.to_owned()).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();
    walk(doc.as_table(), "", text, &mut out);
    Ok(out)
}

pub(crate) fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

fn walk(t: &dyn TableLike, prefix: &str, text: &str, out: &mut HashMap<String, usize>) {
    for (name, item) in t.iter() {
        let path = join(prefix, name);
        if let Some(span) = t.key(name).and_then(toml_edit::Key::span) {
            out.insert(path.clone(), line_of(text, span.start));
        }
        match item {
            Item::ArrayOfTables(aot) => {
                for (i, table) in aot.iter().enumerate() {
                    let p = format!("{path}[{i}]");
                    if let Some(span) = table.span() {
                        out.insert(p.clone(), line_of(text, span.start));
                    }
                    walk(table, &p, text, out);
                }
            }
            Item::Value(Value::Array(arr)) => {
                for (i, v) in arr.iter().enumerate() {
                    if let Some(it) = v.as_inline_table() {
                        let p = format!("{path}[{i}]");
                        if let Some(span) = v.span() {
                            out.insert(p.clone(), line_of(text, span.start));
                        }
                        walk(it, &p, text, out);
                    }
                }
            }
            other => {
                if let Some(tl) = other.as_table_like() {
                    walk(tl, &path, text, out);
                }
            }
        }
    }
}
