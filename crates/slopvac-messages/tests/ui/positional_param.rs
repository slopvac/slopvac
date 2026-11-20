#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// Placeholders must be named.
    #[code("positional-param")]
    #[message("cannot read {}")]
    Oops { file: String },
}

fn main() {}
