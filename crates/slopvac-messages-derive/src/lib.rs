//! Derive macro that turns an error enum into a message catalog for `slopvac-messages`.
//!
//! Applying `#[derive(Catalog)]` to an enum generates its `Catalog` and `Display`
//! implementations. Both render through the one function in `slopvac-messages`, so the
//! macro never writes message text into a per-variant `Display` body. It runs beside
//! `thiserror::Error`, which still supplies `Error`, `source` and `From`.
//!
//! Every variant carries:
//!
//! - `#[code("kebab-case")]`, a stable code that is also a valid Fluent message id, or
//!   `#[code(delegate)]` on a variant that wraps another catalog in its one field.
//! - one of `#[message("…")]` (an error), `#[notice("…")]` (a notice) or `#[text("…")]`
//!   (help or label text). The template names its parameters as `{field}`; `{{` and `}}`
//!   are literal braces.
//! - a doc comment, whose first paragraph becomes the entry summary.
//!
//! Variants are unit or struct variants. A field is a parameter unless it has
//! `#[source]`, `#[from]` or `#[backtrace]`. Mark a count with `#[count]` and a
//! path shown to the user with `#[display_path]`.
//!
//! Every mistake is a compile error. The `slopvac-messages` crate documents the
//! generated API and holds the compile-fail tests.

mod expand;
mod model;
mod template;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derive `Catalog` and `Display` for an enum of messages.
#[proc_macro_derive(Catalog, attributes(code, message, notice, text, count, display_path))]
pub fn derive_catalog(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::Catalog::parse(&input)
        .map_or_else(syn::Error::into_compile_error, |catalog| {
            expand::expand(&catalog)
        })
        .into()
}
