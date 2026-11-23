//! Write the `--preserve-behavior` expansion into the migrated document.
//!
//! [`crate::preserve::expand`] works on 2.x names. This module reads them from the
//! original document, checks them against the bundled 2.x data, and patches the
//! migrated document: it keeps every authored entry with its comments, adds the
//! entries the expansion adds, removes the ones it drops, sets the new level of an
//! entry whose level changes (comments kept), and puts the profile switch blocks
//! before and the guard blocks after the original overrides.

use std::collections::BTreeMap;

use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value, value,
};

use crate::capped::Blockers;
use crate::legacy::{Dials, Layer, Legacy, PROFILES, Scalar, read_dials};
use crate::messages::{MigratePhrase, MigrateText};
use crate::preserve::{Block2, Block3, Config2, DropReason, Expanded, expand};
use crate::{Error, RenameTable};

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
    pub(crate) lines: crate::config::Lines<'a>,
    pub(crate) renames: &'a RenameTable,
    pub(crate) dropped: &'a mut Blockers,
    pub(crate) notes: &'a mut Vec<String>,
}

impl Ctx<'_> {
    fn drop_key(&mut self, path: &str, reason: &str) {
        self.dropped
            .record(self.file, path, self.lines.of(path), reason);
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
        line: ctx.lines.of(&format!("overrides[{}]", e.block)),
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
    let before = ctx.dropped.total();
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
            blocklist: has_blocklist(t),
            profile: read_profile(t, &format!("{path}.profile"), ctx),
            layer: read_layer(t, &path, legacy, ctx),
        });
    }
    (ctx.dropped.total() == before).then_some(Config2 {
        profile,
        blocklist: has_blocklist(doc.as_table()),
        layer,
        overrides,
    })
}

/// Whether the 2.x table sets `vocabulary.path`, which the plain migration moves into
/// `terms.blocklists`.
fn has_blocklist(t: &dyn TableLike) -> bool {
    t.get("vocabulary")
        .and_then(Item::as_table_like)
        .is_some_and(|v| v.contains_key("path"))
}

fn read_profile(t: &dyn TableLike, path: &str, ctx: &mut Ctx<'_>) -> Option<String> {
    let item = t.get("profile")?;
    match item.as_str() {
        Some(p) if PROFILES.contains(&p) => Some(p.to_owned()),
        _ => {
            ctx.drop_key(path, MigratePhrase::ReasonUnknownProfile.as_static());
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
            if ctx.renames.is_removed_category(name) {
                // Dropped with a note by the plain migration; nothing to expand.
            } else if ctx.renames.category(name).is_none() {
                ctx.drop_key(&path, MigratePhrase::ReasonRejectedCategory.as_static());
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
                ctx.drop_key(&path, MigratePhrase::ReasonRejectedRule.as_static());
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
            DropReason::Judgement => MigrateText::NoteDroppedJudgement { path }.to_string(),
            DropReason::Ignored => MigrateText::NoteDroppedIgnored { path }.to_string(),
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
    ctx.notes.push(
        MigrateText::NoteAddedStyle {
            style,
            profile: config.profile.clone(),
        }
        .to_string(),
    );
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
        .map(|b| block_table(b, MigratePhrase::CommentSwitchBlocks.as_static(), ctx))
        .collect();
    let guards: Vec<Table> = expanded
        .guards
        .iter()
        .map(|b| block_table(b, MigratePhrase::CommentGuardBlocks.as_static(), ctx))
        .collect();
    if !switches.is_empty() {
        ctx.notes.push(
            MigrateText::NoteAddedSwitchBlocks {
                blocks: override_blocks(switches.len()),
            }
            .to_string(),
        );
    }
    if !guards.is_empty() {
        ctx.notes.push(
            MigrateText::NoteAddedGuardBlocks {
                blocks: override_blocks(guards.len()),
            }
            .to_string(),
        );
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
        MigratePhrase::OverrideBlockOne.as_static().to_owned()
    } else {
        MigrateText::OverrideBlocks { count: n }.to_string()
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
        .filter(|(qid, _)| !ctx.renames.is_removed(qid))
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
        match rules.get_mut(key).and_then(Item::as_value_mut) {
            None => {
                rules.insert(key, Item::Value(level.as_str().into()));
                added += 1;
            }
            Some(current) if current.as_str() != Some(level.as_str()) => {
                // Keep the comments and spacing around the old value.
                let decor = current.decor().clone();
                *current = level.as_str().into();
                *current.decor_mut() = decor;
                let at = if prefix.is_empty() {
                    format!("rules.{key}")
                } else {
                    format!("{prefix}.rules.{key}")
                };
                ctx.notes.push(
                    MigrateText::NoteSetRuleLevel {
                        at,
                        level: level.clone(),
                    }
                    .to_string(),
                );
            }
            Some(_) => {}
        }
    }
    if added > 0 {
        let at = if prefix.is_empty() {
            MigratePhrase::PlaceTopLevel.as_static().to_owned()
        } else {
            prefix.to_owned()
        };
        ctx.notes.push(
            if added == 1 {
                MigrateText::NoteAddedRuleEntry { count: added, at }
            } else {
                MigrateText::NoteAddedRuleEntries { count: added, at }
            }
            .to_string(),
        );
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
    let wanted = wanted_rules(&block.layer, ctx);
    if !wanted.is_empty() {
        let mut rules = Table::new();
        for (id, level) in &wanted {
            rules.insert(id, value(level.as_str()));
        }
        t.insert("rules", Item::Table(rules));
    }
    t
}
