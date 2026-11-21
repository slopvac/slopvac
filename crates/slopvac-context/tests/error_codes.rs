//! Every `Error` maps to one stable code of the message registry.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use rstest::rstest;
use slopvac_context::{Error, MAX_PATTERN_BYTES, Pattern, TaggerSpec};

#[test]
fn a_tagger_pattern_over_the_cap_has_its_own_code_and_unchanged_text() {
    let error = Error::PatternTooLong {
        len: 9000,
        max: MAX_PATTERN_BYTES,
    };
    assert_eq!(error.code(), "tagger-pattern-too-long");
    assert_eq!(
        error.to_string(),
        "pattern is 9000 bytes, over the 8192-byte limit"
    );
}

fn glob_error() -> globset::Error {
    globset::Glob::new("[").unwrap_err()
}

fn ignore_error() -> ignore::Error {
    ignore::Error::Io(std::io::Error::other("refused"))
}

#[rstest]
#[case::pattern(Pattern::new("(").unwrap_err(), "context-pattern-invalid")]
#[case::match_failed(
    Error::Match {
        pattern: "a".into(),
        source: slopvac_pcre2::MatchError::MatchLimit { limit: 1000 },
    },
    "context-match-failed"
)]
#[case::duplicate_tagger(Error::DuplicateTagger("list".into()), "context-tagger-duplicate")]
#[case::tag_limit(Error::TagLimit { limit: 1 }, "context-tag-limit")]
#[case::unknown_genre(Error::UnknownGenre("memo".into()), "context-genre-unknown")]
#[case::yaml(TaggerSpec::from_yaml("[").unwrap_err(), "context-yaml-invalid")]
#[case::glob(
    Error::Glob { pattern: "[".into(), source: ignore_error() },
    "context-glob-invalid"
)]
#[case::glob_set(Error::GlobSet(ignore_error()), "context-glob-set-invalid")]
#[case::signal_glob(
    Error::SignalGlob { genre: "memo".into(), glob: "[".into(), source: glob_error() },
    "context-signal-glob-invalid"
)]
#[case::read(
    Error::Read { path: "a.md".into(), source: std::io::Error::other("denied") },
    "context-read-failed"
)]
#[case::too_large(Error::TooLarge { path: "a.md".into(), cap: 7 }, "context-file-too-large")]
#[case::unreadable(Error::Unreadable { path: "a.md".into() }, "context-file-unreadable")]
#[case::parse(Error::Parse("bad file".into()), "context-data-invalid")]
fn each_other_error_has_its_published_code(#[case] error: Error, #[case] code: &str) {
    assert_eq!(error.code(), code);
}
