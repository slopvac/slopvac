#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// The template has a lone brace.
    #[code("unclosed-brace")]
    #[message("cannot read {path")]
    Oops { path: String },
}

fn main() {}
