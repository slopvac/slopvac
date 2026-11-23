//! Write the `--preserve-behavior` expansion into the migrated document.
//!
//! [`crate::preserve::expand`] works on 2.x names. This module reads them from the
//! original document, checks them against the bundled 2.x data, and patches the
//! migrated document: it keeps every authored entry with its comments, adds the
//! entries the expansion adds, removes the ones it drops, sets the new level of an
//! entry whose level changes (comments kept), folds each profile switch into the
//! override block that selects the profile, and puts the guard blocks that no block
//! can hold after the original overrides.

use std::collections::BTreeMap;

use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value, value,
};

use crate::capped::Blockers;
use crate::legacy::{Dials, Layer, Legacy, PROFILES, Scalar, read_dials};
use crate::messages::{MigratePhrase, MigrateText};
use crate::preserve::{
    Block2, Block3, Config2, DropReason, Expanded, Out, Patched, category_under_switch, expand,
};
use crate::{Error, RenameTable, overrides};

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
    pub(crate) checked: overrides::Checked,
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
    let blocklist = has_blocklist(original.as_table());
    check_blocklist_layout(original, &config, blocklist, ctx)?;
    let expanded = expand(&config, &legacy).map_err(|e| Error::OverrideOverlap {
        file: ctx.file.to_owned(),
        line: ctx.lines.of(&format!("overrides[{}]", e.block)),
        block: e.block,
        reason: e.reason,
    })?;
    if let Some(found) = category_under_switch(&config) {
        let at = found
            .block
            .map_or_else(|| "categories".to_owned(), |b| format!("overrides[{b}]"));
        return Err(Error::CategoryUnderSwitch {
            file: ctx.file.to_owned(),
            line: ctx.lines.of(&at),
            block: found.block,
            switch: found.switch,
        });
    }
    write(doc, &config, &expanded, blocklist, ctx);
    Ok(())
}

/// Whether the 2.x table sets `vocabulary.path`, which the plain migration moves into
/// `terms.blocklists`.
fn has_blocklist(t: &dyn TableLike) -> bool {
    t.get("vocabulary")
        .and_then(Item::as_table_like)
        .is_some_and(|v| v.contains_key("path"))
}

/// Refuse the layouts where the companion style cannot keep the 2.x vocabulary levels: a
/// blocklist that only an override sets, and a blocklist together with any profile switch. A
/// switch block restates the category levels of the target profile, and those outrank the
/// companion style's rule levels.
fn check_blocklist_layout(
    original: &DocumentMut,
    config: &Config2,
    blocklist: bool,
    ctx: &Ctx<'_>,
) -> Result<(), Error> {
    let refuse = |path: String, reason: String| Error::BlocklistLayout {
        file: ctx.file.to_owned(),
        line: ctx.lines.of(&path),
        reason,
    };
    let blocks = overrides::tables(original.as_table());
    if !blocklist && let Some(i) = blocks.iter().position(|t| has_blocklist(*t)) {
        let at = format!("overrides[{i}].vocabulary.path");
        let reason = MigrateText::ReasonBlocklistInOverride {
            at: at.clone(),
            line: ctx.lines.of(&at).to_string(),
        }
        .to_string();
        return Err(refuse(at, reason));
    }
    if blocklist {
        for (i, block) in config.overrides.iter().enumerate() {
            if block.profile.is_some() {
                let at = format!("overrides[{i}].profile");
                let reason = MigrateText::ReasonBlocklistUnderSwitch {
                    at: at.clone(),
                    line: ctx.lines.of(&at).to_string(),
                }
                .to_string();
                return Err(refuse(at, reason));
            }
        }
    }
    Ok(())
}

/// Read the 2.x settings the expansion uses. Returns `None` after recording an
/// unknown profile, category or rule.
fn read_config(doc: &DocumentMut, legacy: &Legacy, ctx: &mut Ctx<'_>) -> Option<Config2> {
    let before = ctx.dropped.total();
    let profile =
        read_profile(doc.as_table(), "profile", ctx).unwrap_or_else(|| DEFAULT_PROFILE.to_owned());
    let layer = read_layer(doc.as_table(), "", legacy, ctx);
    let mut overrides = Vec::new();
    for (i, t) in overrides::tables(doc.as_table()).into_iter().enumerate() {
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
    (ctx.dropped.total() == before).then_some(Config2 {
        profile,
        layer,
        overrides,
        top_names_category: doc
            .get("categories")
            .and_then(Item::as_table_like)
            .is_some_and(|t| !t.is_empty()),
    })
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

/// Set `profile` and list the legacy style of the profile in `styles`, followed by its
/// vocabulary companion when the config moved a word blocklist.
fn add_styles(doc: &mut DocumentMut, config: &Config2, blocklist: bool, ctx: &mut Ctx<'_>) {
    let style = format!("{STYLE_PREFIX}{}", config.profile);
    if !doc.contains_key("profile") {
        doc.insert("profile", value(&config.profile));
    }
    let mut styles = Array::new();
    styles.push(style.as_str());
    let companion = format!("{style}-vocabulary");
    if blocklist {
        styles.push(companion.as_str());
    }
    doc.insert("styles", value(styles));
    ctx.notes.push(
        MigrateText::NoteAddedStyle {
            style,
            profile: config.profile.clone(),
        }
        .to_string(),
    );
    if blocklist {
        ctx.notes.push(
            MigrateText::NoteAddedVocabularyStyle {
                style: companion,
                profile: config.profile.clone(),
            }
            .to_string(),
        );
    }
}

fn write(
    doc: &mut DocumentMut,
    config: &Config2,
    expanded: &Expanded,
    blocklist: bool,
    ctx: &mut Ctx<'_>,
) {
    add_styles(doc, config, blocklist, ctx);
    note_dropped(expanded, ctx);
    let first_override = first_override_position(doc);
    patch_layer(
        doc.as_table_mut(),
        "",
        &expanded.top,
        first_override.map(|p| p - 1),
        ctx,
    );
    note_layout(expanded, ctx);
    match overrides::item_mut(ctx.checked, doc.as_table_mut()) {
        Some(Item::ArrayOfTables(aot)) => {
            let originals: Vec<Table> = aot.iter().cloned().collect();
            let views: Vec<&dyn TableLike> =
                originals.iter().map(|t| t as &dyn TableLike).collect();
            let mut pool = Vec::new();
            for t in &originals {
                positions(t, &mut pool);
            }
            pool.sort_unstable();
            let mut rebuilt = ArrayOfTables::new();
            for out in &expanded.overrides {
                rebuilt.push(match out {
                    Out::Original(b) => {
                        let mut t = originals[b.index].clone();
                        patch_block(&mut t, b, false, &views, ctx);
                        t
                    }
                    Out::Guard(g) => {
                        block_table(g, MigratePhrase::CommentGuardBlocks.as_static(), ctx)
                    }
                });
            }
            // Moved blocks take the document positions in their new order, so they
            // render in it.
            let mut next = pool.into_iter();
            for t in rebuilt.iter_mut() {
                reposition(t, &mut next);
            }
            *aot = rebuilt;
        }
        Some(item) => {
            let Some(arr) = item.as_array_mut() else {
                return;
            };
            let originals: Vec<InlineTable> = arr
                .iter()
                .filter_map(Value::as_inline_table)
                .cloned()
                .collect();
            let views: Vec<&dyn TableLike> =
                originals.iter().map(|t| t as &dyn TableLike).collect();
            let mut values = Vec::new();
            for out in &expanded.overrides {
                values.push(match out {
                    Out::Original(b) => {
                        let mut t = originals[b.index].clone();
                        patch_block(&mut t, b, true, &views, ctx);
                        (Value::InlineTable(t), true)
                    }
                    Out::Guard(g) => (
                        Value::InlineTable(
                            block_table(g, MigratePhrase::CommentGuardBlocks.as_static(), ctx)
                                .into_inline_table(),
                        ),
                        false,
                    ),
                });
            }
            arr.clear();
            for (v, formatted) in values {
                if formatted {
                    arr.push_formatted(v);
                } else {
                    arr.push(v);
                }
            }
        }
        None => {}
    }
}

/// The notes for the new order of the override blocks: each folded switch, each restated
/// or moved block, each guard folded into a block, and the guard blocks added at the end.
fn note_layout(expanded: &Expanded, ctx: &mut Ctx<'_>) {
    let list = |blocks: &[usize]| {
        blocks
            .iter()
            .map(|j| format!("overrides[{j}]"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut guards = 0;
    for out in &expanded.overrides {
        let b = match out {
            Out::Original(b) => b,
            Out::Guard(_) => {
                guards += 1;
                continue;
            }
        };
        let at = format!("overrides[{}]", b.index);
        if let Some(profile) = &b.switch {
            ctx.notes.push(
                MigrateText::NoteMergedSwitch {
                    profile: profile.clone(),
                    at: at.clone(),
                }
                .to_string(),
            );
            if !b.restated.is_empty() {
                ctx.notes.push(
                    MigrateText::NoteRestatedBlocks {
                        at: at.clone(),
                        blocks: list(&b.restated),
                    }
                    .to_string(),
                );
            }
        }
        if b.guarded > 0 {
            let count = b.guarded;
            let at = at.clone();
            ctx.notes.push(
                if count == 1 {
                    MigrateText::NoteGuardedRuleEntry { count, at }
                } else {
                    MigrateText::NoteGuardedRuleEntries { count, at }
                }
                .to_string(),
            );
        }
        if let Some(after) = b.moved_after {
            ctx.notes.push(
                MigrateText::NoteMovedBlock {
                    at,
                    after: format!("overrides[{after}]"),
                    blocks: list(&b.yields_to),
                }
                .to_string(),
            );
        }
    }
    if guards > 0 {
        ctx.notes.push(
            MigrateText::NoteAddedGuardBlocks {
                blocks: override_blocks(guards),
            }
            .to_string(),
        );
    }
}

/// Collect the document positions of `t` and of the tables inside it.
fn positions(t: &Table, out: &mut Vec<isize>) {
    out.extend(t.position());
    for (_, item) in t {
        match item {
            Item::Table(sub) => positions(sub, out),
            Item::ArrayOfTables(aot) => aot.iter().for_each(|sub| positions(sub, out)),
            _ => {}
        }
    }
}

/// Give `t` and the tables inside it that have a position the next ones from `next`.
fn reposition(t: &mut Table, next: &mut impl Iterator<Item = isize>) {
    if t.position().is_some() {
        t.set_position(next.next());
    }
    for (_, item) in t.iter_mut() {
        match item {
            Item::Table(sub) => reposition(sub, next),
            Item::ArrayOfTables(aot) => aot.iter_mut().for_each(|sub| reposition(sub, next)),
            _ => {}
        }
    }
}

/// An empty sub-table in the form of its block.
fn sub_table(inline: bool) -> Item {
    if inline {
        Item::Value(Value::InlineTable(InlineTable::new()))
    } else {
        Item::Table(Table::new())
    }
}

/// Make an original block hold `b.layer`, keeping the comments of the entries that stay,
/// and drop the other settings that a block it now sits above sets.
fn patch_block(
    t: &mut dyn TableLike,
    b: &Patched,
    inline: bool,
    originals: &[&dyn TableLike],
    ctx: &mut Ctx<'_>,
) {
    let prefix = format!("overrides[{}]", b.index);
    let cats: BTreeMap<String, Dials> = b
        .layer
        .categories
        .iter()
        .map(|(name, d)| (ctx.category(name), d.clone()))
        .collect();
    reconcile_categories(t, &cats, inline);
    let wanted = wanted_rules(&b.layer, ctx);
    if !t.contains_key("rules") && !wanted.is_empty() {
        t.insert("rules", sub_table(inline));
    }
    if let Some(rules) = t.get_mut("rules").and_then(Item::as_table_like_mut) {
        let plain = b.switch.is_none() && b.moved_after.is_none() && b.guarded == 0;
        reconcile_rules(rules, &wanted, &prefix, plain, ctx);
        if rules.is_empty() && !plain {
            t.remove("rules");
        }
    }
    for &k in &b.yields_to {
        for (key, item) in originals[k].iter() {
            if !matches!(key, "files" | "categories" | "rules") {
                remove_leaves(t, key, item);
            }
        }
    }
}

/// Remove from `t` every leaf of `over` at `key`.
fn remove_leaves(t: &mut dyn TableLike, key: &str, over: &Item) {
    let Some(mine) = t.get_mut(key) else {
        return;
    };
    match (mine.as_table_like_mut(), over.as_table_like()) {
        (Some(mine), Some(theirs)) => {
            for (k, v) in theirs.iter() {
                remove_leaves(mine, k, v);
            }
            if mine.is_empty() {
                t.remove(key);
            }
        }
        _ => {
            t.remove(key);
        }
    }
}

/// Replace a value, keeping the comments and spacing around it.
fn set_value(current: &mut Value, new: Value) {
    let decor = current.decor().clone();
    *current = new;
    *current.decor_mut() = decor;
}

/// Make the `categories` of a block hold exactly `wanted` (3.0 names).
fn reconcile_categories(t: &mut dyn TableLike, wanted: &BTreeMap<String, Dials>, inline: bool) {
    if !t.contains_key("categories") {
        if wanted.is_empty() {
            return;
        }
        t.insert("categories", sub_table(inline));
    }
    let Some(cats) = t.get_mut("categories").and_then(Item::as_table_like_mut) else {
        return;
    };
    let existing: Vec<String> = cats.iter().map(|(k, _)| k.to_owned()).collect();
    for key in &existing {
        if !wanted.contains_key(key) {
            cats.remove(key);
        }
    }
    for (name, d) in wanted {
        match cats.get_mut(name) {
            None => {
                cats.insert(name, Item::Value(dials_value(d)));
            }
            Some(item) => {
                if read_dials(item).as_ref() == Some(d) {
                    continue;
                }
                if let Some(table) = item.as_table_like_mut() {
                    reconcile_dials(table, d);
                } else if let Some(current) = item.as_value_mut() {
                    set_value(current, dials_value(d));
                } else {
                    *item = Item::Value(dials_value(d));
                }
            }
        }
    }
    if cats.is_empty() {
        t.remove("categories");
    }
}

fn reconcile_dials(t: &mut dyn TableLike, d: &Dials) {
    let existing: Vec<String> = t.iter().map(|(k, _)| k.to_owned()).collect();
    for key in &existing {
        if !d.contains_key(key) {
            t.remove(key);
        }
    }
    for key in DIAL_ORDER {
        let Some(v) = d.get(key) else {
            continue;
        };
        match t.get_mut(key) {
            Some(item) if Scalar::from_item(item).as_ref() == Some(v) => {}
            Some(item) => match item.as_value_mut() {
                Some(current) => set_value(current, v.to_value()),
                None => *item = Item::Value(v.to_value()),
            },
            None => {
                t.insert(key, Item::Value(v.to_value()));
            }
        }
    }
}

/// The document position of the first `[[overrides]]` table. New tables placed
/// relative to it render in front of the original overrides.
fn first_override_position(doc: &DocumentMut) -> Option<isize> {
    match overrides::item(doc.as_table()) {
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
        reconcile_rules(rules, &wanted, prefix, true, ctx);
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
    plain: bool,
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
                    if plain {
                        MigrateText::NoteSetRuleLevel {
                            at,
                            level: level.clone(),
                        }
                    } else {
                        MigrateText::NoteSetRuleLevelApplied {
                            at,
                            level: level.clone(),
                        }
                    }
                    .to_string(),
                );
            }
            Some(_) => {}
        }
    }
    if added > 0 && plain {
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
