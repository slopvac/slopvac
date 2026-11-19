#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// Not kebab-case.
    #[code("Bad_Code")]
    #[message("oops")]
    Oops,
}

fn main() {}
