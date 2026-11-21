//! Each `StyleError` variant renders the text published for it.

use rstest::rstest;
use slopvac_styles::{Conflict, StyleError};

fn s(text: &str) -> String {
    text.to_owned()
}

#[rstest]
#[case::not_found(StyleError::NotFound(s("calm")), "unknown style `calm`")]
#[case::invalid_name(StyleError::InvalidName(s("../x")), "invalid style name `../x`")]
#[case::parse(
    StyleError::Parse { style: s("calm"), message: s("bad") },
    "style `calm`: bad"
)]
#[case::invalid(
    StyleError::Invalid {
        style: s("calm"),
        problems: vec![(s("config-unknown-key"), s("/foo"))],
        rendered: s("line 1"),
    },
    "style `calm` is invalid:\nline 1"
)]
#[case::cycle(
    StyleError::Cycle(vec![s("a"), s("b"), s("a")]),
    "style extends cycle: a -> b -> a"
)]
#[case::conflict(
    StyleError::Conflict { loser: s("a"), winner: s("b") },
    "style `a` conflicts with `b`; `b` wins"
)]
#[case::too_large(
    StyleError::TooLarge { style: s("calm"), cap: 1_048_576 },
    "[style-too-large] style `calm` is larger than 1048576 bytes"
)]
#[case::io(
    StyleError::Io { style: s("calm"), message: s("denied") },
    "reading style `calm`: denied"
)]
fn each_error_renders_its_published_text(#[case] error: StyleError, #[case] expected: &str) {
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::no_keys(vec![], "style `a` conflicts with `b`; `b` wins")]
#[case::keys(
    vec![s("rules.core:list"), s("terms.allow")],
    "style `a` conflicts with `b`; `b` wins on rules.core:list, terms.allow"
)]
fn a_conflict_warning_names_the_winner_and_the_keys(
    #[case] keys: Vec<String>,
    #[case] expected: &str,
) {
    let conflict = Conflict {
        loser: s("a"),
        winner: s("b"),
        keys,
    };
    assert_eq!(conflict.message(), expected);
}
