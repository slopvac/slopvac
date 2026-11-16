#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Bad {
    /// Starts with a digit.
    #[code("1st-error")]
    #[message("oops")]
    Oops,
}

fn main() {}
