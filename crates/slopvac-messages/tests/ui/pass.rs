#![allow(dead_code)]
use slopvac_messages::Catalog;

#[derive(Catalog)]
enum Ok {
    /// A fine message.
    #[code("fine-message")]
    #[message("hello {name}")]
    Hello { name: String },
}

fn main() {}
