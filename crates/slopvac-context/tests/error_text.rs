//! `Error::raw_message` is the message before escaping; `Display` is the same message escaped.

use rstest::rstest;
use slopvac_context::Error;

/// A hostile id: a backslash, a newline and a bidi override.
const HOSTILE: &str = "a\\b\n\u{202e}c";
/// `HOSTILE` as `Display` shows it.
const ESCAPED: &str = "a\\\\b\\u{a}\\u{202e}c";

#[rstest]
#[case::unknown_genre(Error::UnknownGenre(HOSTILE.into()), "unknown genre `{}`")]
#[case::duplicate_tagger(Error::DuplicateTagger(HOSTILE.into()), "tagger `{}` is already registered")]
#[case::unreadable(Error::Unreadable { path: HOSTILE.into() }, "{}: cannot read")]
#[case::too_large(
    Error::TooLarge { path: HOSTILE.into(), cap: 7 },
    "{}: larger than the 7-byte document limit"
)]
#[case::parse(Error::Parse(HOSTILE.into()), "{}")]
#[case::read(
    Error::Read { path: HOSTILE.into(), source: std::io::Error::other(HOSTILE) },
    "{}: {}"
)]
fn raw_message_keeps_the_text_and_display_escapes_it_once(
    #[case] error: Error,
    #[case] template: &str,
) {
    let fill = |text: &str| template.replacen("{}", text, template.matches("{}").count());
    assert_eq!(error.raw_message(), fill(HOSTILE));
    assert_eq!(error.to_string(), fill(ESCAPED));
}
