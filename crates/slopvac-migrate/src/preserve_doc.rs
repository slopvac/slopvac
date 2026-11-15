//! Write the `--preserve-behavior` expansion into the migrated document.
//!
//! [`crate::preserve::expand`] works on 2.x names. This module reads them from the
//! original document, checks them against the bundled 2.x data, and patches the
//! migrated document: it keeps every authored entry with its comments, adds the
//! entries the expansion adds, removes the ones it drops, and puts the profile
//! switch blocks before and the guard blocks after the original overrides.
// STUB(slopvac-8l4.16.11): follows the expansion of PR #296 b138f5888a.

use std::collections::{BTreeMap, HashMap};

use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value, value,
};

use crate::legacy::{Dials, Layer, Legacy, PROFILES, Scalar, read_dials};
use crate::preserve::{Block2, Block3, Config2, DropReason, Expanded, expand};
use crate::{Dropped, Error, RenameTable};

/// The pack that ships the legacy styles (docs/contract/profiles.md, E1).
const STYLE_PREFIX: &str = "core:legacy-";
/// The 2.x default profile.
const DEFAULT_PROFILE: &str = "normal";
/// The order dials are written in.
const DIAL_ORDER: [&str; 4] = [
    "severity",
    "minimum_severity",
    "max_per_100_words",
    "weight",
];

/// What the patch needs besides the documents.
pub(crate) struct Ctx<'a> {
    pub(crate) file: &'a str,
    pub(crate) lines: &'a HashMap<String, usize>,
    pub(crate) renames: &'a RenameTable,
    pub(crate) dropped: &'a mut Vec<Dropped>,
    pub(crate) notes: &'a mut Vec<String>,
}

impl Ctx<'_> {
    fn drop_key(&mut self, path: &str, reason: &str) {
        self.dropped.push(Dropped {
            file: self.file.to_owned(),
            key: path.to_owned(),
            line: self.lines.get(path).copied().unwrap_or(1),
            reason: reason.to_owned(),
        });
    }

    fn rule(&self, qid: &str) -> String {
        self.renames.rule(qid).unwrap_or_else(|| qid.to_owned())
    }

    fn category(&self, name: &str) -> String {
        self.renames
            .category(name)
            .unwrap_or_else(|| name.to_owned())
    }
}

/// Expand `original` (the 2.x document) and write the result into `doc` (the same
/// document after the plain migration). Unknown names go to `ctx.dropped`.
pub(crate) fn apply(
    original: &DocumentMut,
    doc: &mut DocumentMut,
    ctx: &mut Ctx<'_>,
) -> Result<(), Error> {
    let legacy = Legacy::bundled();
    let Some(config) = read_config(original, &legacy, ctx) else {
        return Ok(());
    };
    let expanded = expand(&config, &legacy).map_err(|e| Error::OverrideOverlap {
        file: ctx.file.to_owned(),
        line: ctx
            .lines
            .get(&format!("overrides[{}]", e.block))
            .copied()
            .unwrap_or(1),
        block: e.block,
        reason: e.reason,
    })?;
    write(doc, &config, &expanded, ctx);
    Ok(())
}

fn override_tables(doc: &DocumentMut) -> Vec<&dyn TableLike> {
    match doc.get("overrides") {
        Some(Item::ArrayOfTables(aot)) => aot.iter().map(|t| t as &dyn TableLike).collect(),
        Some(item) => item
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_inline_table)
                    .map(|t| t as &dyn TableLike)
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Read the 2.x settings the expansion uses. Returns `None` after recording an
/// unknown profile, category or rule.
fn read_config(doc: &DocumentMut, legacy: &Legacy, ctx: &mut Ctx<'_>) -> Option<Config2> {
    let before = ctx.dropped.len();
    let profile =
        read_profile(doc.as_table(), "profile", ctx).unwrap_or_else(|| DEFAULT_PROFILE.to_owned());
    let layer = read_layer(doc.as_table(), "", legacy, ctx);
    let mut overrides = Vec::new();
    for (i, t) in override_tables(doc).into_iter().enumerate() {
        let path = format!("overrides[{i}]");
        let files = t
            .get("files")
            .and_then(Item::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        overrides.push(Block2 {
            files,
            profile: read_profile(t, &format!("{path}.profile"), ctx),
            layer: read_layer(t, &path, legacy, ctx),
        });
    }
    (ctx.dropped.len() == before).then_some(Config2 {
        profile,
        layer,
        overrides,
    })
}

fn read_profile(t: &dyn TableLike, path: &str, ctx: &mut Ctx<'_>) -> Option<String> {
    let item = t.get("profile")?;
    match item.as_str() {
        Some(p) if PROFILES.contains(&p) => Some(p.to_owned()),
        _ => {
            ctx.drop_key(
                path,
                "unknown 2.x profile; expected strict, normal or relaxed",
            );
            None
        }
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn read_layer(t: &dyn TableLike, prefix: &str, legacy: &Legacy, ctx: &mut Ctx<'_>) -> Layer {
    let mut layer = Layer::default();
    if let Some(cats) = t.get("categories").and_then(Item::as_table_like) {
        for (name, item) in cats.iter() {
            let path = join(&join(prefix, "categories"), name);
            if !legacy.style(DEFAULT_PROFILE).categories.contains_key(name) {
                ctx.drop_key(&path, "unknown 2.x category; 2.x rejected it");
            } else if let Some(d) = read_dials(item) {
                layer.categories.insert(name.to_owned(), d);
            }
        }
    }
    if let Some(rules) = t.get("rules").and_then(Item::as_table_like) {
        for (qid, item) in rules.iter() {
            let path = join(&join(prefix, "rules"), qid);
            let level = item.as_str().or_else(|| {
                item.as_table_like()
                    .and_then(|t| t.get("severity"))
                    .and_then(Item::as_str)
            });
            if !legacy.rules.contains_key(qid) {
                ctx.drop_key(&path, "unknown 2.x rule; 2.x rejected it");
            } else if let Some(level) = level {
                layer.rules.insert(qid.to_owned(), level.to_owned());
            }
        }
    }
    layer
}

fn note_dropped(expanded: &Expanded, ctx: &mut Ctx<'_>) {
    for d in &expanded.dropped {
        let path = match d.block {
            Some(i) => format!("overrides[{i}].rules.{}", d.rule),
            None => format!("rules.{}", d.rule),
        };
        ctx.notes.push(match d.reason {
            DropReason::Judgement => format!(
                "dropped {path}: a judgement rule takes its review level from the legacy style"
            ),
            DropReason::Ignored => {
                format!("dropped {path}: 2.x ignored it for every file it applies to")
            }
        });
    }
}

fn write(doc: &mut DocumentMut, config: &Config2, expanded: &Expanded, ctx: &mut Ctx<'_>) {
    let style = format!("{STYLE_PREFIX}{}", config.profile);
    if !doc.contains_key("profile") {
        doc.insert("profile", value(&config.profile));
    }
    let mut styles = Array::new();
    styles.push(style.as_str());
    doc.insert("styles", value(styles));
    ctx.notes.push(format!(
        "added styles = [\"{style}\"] to reproduce the 2.x {} profile",
        config.profile
    ));
    note_dropped(expanded, ctx);
    let first_override = first_override_position(doc);
    patch_layer(
        doc.as_table_mut(),
        "",
        &expanded.top,
        first_override.map(|p| p - 1),
        ctx,
    );
    let inline = !matches!(doc.get("overrides"), Some(Item::ArrayOfTables(_)) | None);
    match doc.get_mut("overrides") {
        Some(Item::ArrayOfTables(aot)) => {
            for (i, (t, layer)) in aot.iter_mut().zip(&expanded.blocks).enumerate() {
                patch_layer(t, &format!("overrides[{i}]"), layer, None, ctx);
            }
        }
        Some(item) => {
            if let Some(arr) = item.as_array_mut() {
                let tables = arr.iter_mut().filter_map(Value::as_inline_table_mut);
                for (i, (t, layer)) in tables.zip(&expanded.blocks).enumerate() {
                    patch_inline_layer(t, &format!("overrides[{i}]"), layer, ctx);
                }
            }
        }
        None => {}
    }
    if expanded.switches.is_empty() && expanded.guards.is_empty() {
        return;
    }
    let switches: Vec<Table> = expanded
        .switches
        .iter()
        .map(|b| {
            block_table(
                b,
                "Added by `slopvac migrate --preserve-behavior`: switch these files to the 2.x\n# profile that the block below with the same files selects.",
                ctx,
            )
        })
        .collect();
    let guards: Vec<Table> = expanded
        .guards
        .iter()
        .map(|b| {
            block_table(
                b,
                "Added by `slopvac migrate --preserve-behavior`: 2.x ignored these rule entries\n# for these files.",
                ctx,
            )
        })
        .collect();
    if !switches.is_empty() {
        ctx.notes.push(format!(
            "added {} before the others that switch files to another 2.x profile",
            override_blocks(switches.len())
        ));
    }
    if !guards.is_empty() {
        ctx.notes.push(format!(
            "added {} after the others that turn off rule entries 2.x ignored",
            override_blocks(guards.len())
        ));
    }
    if inline {
        if let Some(arr) = doc.get_mut("overrides").and_then(Item::as_array_mut) {
            for (i, t) in switches.into_iter().enumerate() {
                arr.insert(i, t.into_inline_table());
            }
            for t in guards {
                arr.push(t.into_inline_table());
            }
        }
        return;
    }
    let Some(Item::ArrayOfTables(aot)) = doc.get_mut("overrides") else {
        return;
    };
    let mut rebuilt = ArrayOfTables::new();
    for mut t in switches {
        // Sort in front of the first original block and after a new top-level table.
        t.set_position(first_override);
        rebuilt.push(t);
    }
    for t in aot.iter() {
        rebuilt.push(t.clone());
    }
    for t in guards {
        rebuilt.push(t);
    }
    *aot = rebuilt;
}

/// The document position of the first `[[overrides]]` table. New tables placed
/// relative to it render in front of the original overrides.
fn first_override_position(doc: &DocumentMut) -> Option<isize> {
    match doc.get("overrides") {
        Some(Item::ArrayOfTables(aot)) => aot.iter().filter_map(Table::position).min(),
        _ => None,
    }
}

fn override_blocks(n: usize) -> String {
    if n == 1 {
        "1 override block".to_owned()
    } else {
        format!("{n} override blocks")
    }
}

fn dials_value(d: &Dials) -> Value {
    if d.len() == 1
        && let Some(Scalar::Str(level)) = d.get("severity")
    {
        return level.as_str().into();
    }
    let mut t = InlineTable::new();
    for key in DIAL_ORDER {
        if let Some(v) = d.get(key) {
            t.insert(key, v.to_value());
        }
    }
    Value::InlineTable(t)
}

/// Add `severity` to authored categories without one (E5) and reconcile `rules`
/// with the expansion (E2, E4) in one table-form layer.
fn patch_layer(
    t: &mut dyn TableLike,
    prefix: &str,
    layer: &Layer,
    position: Option<isize>,
    ctx: &mut Ctx<'_>,
) {
    patch_severity(t, layer, ctx);
    let wanted = wanted_rules(layer, ctx);
    if !t.contains_key("rules") {
        if wanted.is_empty() {
            return;
        }
        let mut table = Table::new();
        if let Some(p) = position {
            table.set_position(Some(p));
        }
        t.insert("rules", Item::Table(table));
    }
    if let Some(rules) = t.get_mut("rules").and_then(Item::as_table_like_mut) {
        reconcile_rules(rules, &wanted, prefix, ctx);
    }
}

fn patch_inline_layer(t: &mut InlineTable, prefix: &str, layer: &Layer, ctx: &mut Ctx<'_>) {
    patch_severity(t, layer, ctx);
    let wanted = wanted_rules(layer, ctx);
    if !t.contains_key("rules") {
        if wanted.is_empty() {
            return;
        }
        t.insert("rules", Value::InlineTable(InlineTable::new()));
    }
    if let Some(rules) = t.get_mut("rules").and_then(Value::as_inline_table_mut) {
        reconcile_rules(rules, &wanted, prefix, ctx);
    }
}

fn patch_severity(t: &mut dyn TableLike, layer: &Layer, ctx: &Ctx<'_>) {
    let Some(cats) = t.get_mut("categories").and_then(Item::as_table_like_mut) else {
        return;
    };
    for (name, d) in &layer.categories {
        let Some(level) = d.get("severity") else {
            continue;
        };
        if let Some(entry) = cats
            .get_mut(&ctx.category(name))
            .and_then(Item::as_table_like_mut)
            && !entry.contains_key("severity")
        {
            entry.insert("severity", Item::Value(level.to_value()));
        }
    }
}

fn wanted_rules(layer: &Layer, ctx: &Ctx<'_>) -> BTreeMap<String, String> {
    layer
        .rules
        .iter()
        .map(|(qid, level)| (ctx.rule(qid), level.clone()))
        .collect()
}

fn reconcile_rules(
    rules: &mut dyn TableLike,
    wanted: &BTreeMap<String, String>,
    prefix: &str,
    ctx: &mut Ctx<'_>,
) {
    let existing: Vec<String> = rules.iter().map(|(k, _)| k.to_owned()).collect();
    for key in &existing {
        if !wanted.contains_key(key) {
            rules.remove(key);
        }
    }
    let mut added = 0;
    for (key, level) in wanted {
        if !rules.contains_key(key) {
            rules.insert(key, Item::Value(level.as_str().into()));
            added += 1;
        }
    }
    if added > 0 {
        let at = if prefix.is_empty() {
            "the top level".to_owned()
        } else {
            prefix.to_owned()
        };
        ctx.notes.push(format!(
            "added {added} rule {} at {at}: 2.x applied these profile defaults over the authored category level",
            if added == 1 { "entry" } else { "entries" }
        ));
    }
}

fn block_table(block: &Block3, comment: &str, ctx: &Ctx<'_>) -> Table {
    let mut t = Table::new();
    t.decor_mut().set_prefix(format!("\n# {comment}\n"));
    let mut files = Array::new();
    for f in &block.files {
        files.push(f.as_str());
    }
    t.insert("files", value(files));
    if !block.layer.categories.is_empty() {
        let mut cats = Table::new();
        for (name, d) in &block.layer.categories {
            cats.insert(&ctx.category(name), Item::Value(dials_value(d)));
        }
        t.insert("categories", Item::Table(cats));
    }
    if !block.layer.rules.is_empty() {
        let mut rules = Table::new();
        for (qid, level) in &block.layer.rules {
            rules.insert(&ctx.rule(qid), value(level.as_str()));
        }
        t.insert("rules", Item::Table(rules));
    }
    t
}
