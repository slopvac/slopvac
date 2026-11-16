//! The catalog trait and the single function that renders every message.

use std::fmt;

use crate::entry::Entry;
use crate::template::{Segment, segments};
use crate::value::{Arg, Value};

/// An enum of messages. Derive it with `#[derive(Catalog)]`.
pub trait Catalog {
    /// Every message this catalog defines. A variant that delegates adds none: the wrapped
    /// catalog lists its own.
    fn entries() -> &'static [Entry]
    where
        Self: Sized;

    /// The message this value renders.
    fn entry(&self) -> &'static Entry;

    /// The parameter values of this message.
    fn args(&self) -> Vec<Arg>;

    /// The stable code of this message.
    fn code(&self) -> &'static str {
        self.entry().code
    }
}

/// Render a message: its template with the parameters filled in.
///
/// Every message goes through this function. `Display` for a catalog calls it, so changing
/// how messages become text means changing this function and nothing else. A placeholder
/// with no matching argument is written as `{name}`.
#[must_use]
pub fn render(catalog: &dyn Catalog) -> String {
    render_entry(catalog.entry(), &catalog.args())
}

pub(crate) fn render_entry(entry: &Entry, args: &[Arg]) -> String {
    let mut out = String::with_capacity(entry.template.len());
    for segment in segments(entry.template) {
        match segment {
            Segment::Literal(text) => out.push_str(text),
            Segment::Placeholder(name) => match args.iter().find(|a| a.name == name) {
                Some(Arg {
                    value: Value::Text(text) | Value::Path(text),
                    ..
                }) => out.push_str(text),
                Some(Arg {
                    value: Value::Count(n),
                    ..
                }) => out.push_str(&n.to_string()),
                _ => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            },
        }
    }
    out
}

/// Write a rendered message to a formatter. The derived `Display` calls this.
///
/// # Errors
///
/// Returns the formatter's error.
pub fn write_message(catalog: &dyn Catalog, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&render(catalog))
}
