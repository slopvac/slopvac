#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// Parameters must be named.
    #[code("tuple-variant")]
    #[message("cannot read")]
    Oops(String),
}

fn main() {}
