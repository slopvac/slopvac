//! A field named `source` is the error cause even without an attribute.

mod support;

use std::error::Error as _;

use slopvac_messages::{Catalog, Param, ParamKind};
use support::Implicit;

fn step() -> Implicit {
    Implicit::Step {
        step: "build".into(),
        source: std::io::Error::other("disk full"),
    }
}

#[test]
fn the_source_field_is_not_a_message_parameter() {
    assert_eq!(step().entry().params, [Param::new("step", ParamKind::Text)]);
    assert_eq!(step().args().len(), 1);
}

#[test]
fn rendering_does_not_need_the_source_field() {
    assert_eq!(step().to_string(), "step build failed");
}

#[test]
fn error_source_returns_the_source_field() {
    assert_eq!(step().source().expect("a source").to_string(), "disk full");
}
