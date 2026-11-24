//! The one place that reads the 2.x `overrides` list.
//!
//! 2.x reads `overrides` as a list of tables. [`check`] refuses any other shape before the
//! migration rewrites anything, so no rewrite drops an entry it cannot read. Every other
//! module reaches the list through this one: [`item`] and [`tables`] read it, and
//! [`item_mut`] and [`tables_mut`] change it only with the [`Checked`] that [`check`]
//! returns.

use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::Error;
use crate::config::Lines;

/// The key of the list.
const KEY: &str = "overrides";

/// Proof that [`check`] accepted the shape of `overrides`. Only [`check`] makes one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Checked(());

/// Accept `overrides` when it is absent or a list whose entries are all tables. Otherwise
/// refuse with [`Error::OverrideShape`], which names the first entry, or the list itself,
/// that is not a table.
pub(crate) fn check(doc: &DocumentMut, file: &str, lines: Lines<'_>) -> Result<Checked, Error> {
    let at = match doc.get(KEY) {
        None | Some(Item::ArrayOfTables(_)) => None,
        Some(Item::Value(Value::Array(arr))) => arr
            .iter()
            .position(|v| v.as_inline_table().is_none())
            .map(|i| format!("{KEY}[{i}]")),
        Some(_) => Some(KEY.to_owned()),
    };
    match at {
        None => Ok(Checked(())),
        Some(at) => Err(Error::OverrideShape {
            file: file.to_owned(),
            line: lines.of(&at),
            at,
        }),
    }
}

/// The `overrides` item of `t`, to read.
pub(crate) fn item(t: &dyn TableLike) -> Option<&Item> {
    t.get(KEY)
}

/// The `overrides` item of `t`, to change.
pub(crate) fn item_mut(_: Checked, t: &mut dyn TableLike) -> Option<&mut Item> {
    t.get_mut(KEY)
}

/// The override tables of `t`, in order.
pub(crate) fn tables(t: &dyn TableLike) -> Vec<&dyn TableLike> {
    match item(t) {
        Some(Item::ArrayOfTables(aot)) => aot.iter().map(|t| t as &dyn TableLike).collect(),
        Some(Item::Value(Value::Array(arr))) => arr
            .iter()
            .filter_map(|v| Some(v.as_inline_table()? as &dyn TableLike))
            .collect(),
        _ => Vec::new(),
    }
}

/// The override tables of `t`, in order, to change.
pub(crate) fn tables_mut(checked: Checked, t: &mut dyn TableLike) -> Vec<&mut dyn TableLike> {
    match item_mut(checked, t) {
        Some(Item::ArrayOfTables(aot)) => aot.iter_mut().map(|t| t as &mut dyn TableLike).collect(),
        Some(Item::Value(Value::Array(arr))) => arr
            .iter_mut()
            .filter_map(|v| Some(v.as_inline_table_mut()? as &mut dyn TableLike))
            .collect(),
        _ => Vec::new(),
    }
}
