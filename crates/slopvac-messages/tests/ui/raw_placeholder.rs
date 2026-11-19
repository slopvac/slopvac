#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// Placeholders use the plain name.
    #[code("raw-placeholder")]
    #[message("unknown {r#type}")]
    Oops { r#type: String },
}

fn main() {}
