#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// The template names a field that does not exist.
    #[code("missing-field")]
    #[message("cannot read {path}")]
    Oops { file: String },
}

fn main() {}
