//! The messages `slopvac-styles` reports, as one catalog that the binary registers through
//! [`message_entries`].
//!
//! [`StyleMessage`] holds the text of each [`crate::StyleError`] variant, the conflict
//! warning, the detection lines `slopvac init` prints and the sentences a preset file reports
//! while it parses. The detection notice for an oversized file carries its stable code
//! `style-detect-file-too-large`. The code `style-too-large` belongs to the config catalog,
//! so [`StyleMessage::TooLarge`] takes it as a parameter.

use slopvac_config::Code;
use slopvac_messages::{Catalog, Entry};

/// Every sentence `slopvac-styles` produces for a user.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum StyleMessage {
    /// No style has this name.
    #[code("style-error-not-found")]
    #[text("unknown style `{name}`")]
    NotFound { name: String },
    /// The name is not a plain style name.
    #[code("style-error-invalid-name")]
    #[text("invalid style name `{name}`")]
    InvalidName { name: String },
    /// A style file is not valid TOML or does not match the shape.
    #[code("style-error-parse")]
    #[text("style `{style}`: {message}")]
    Parse { style: String, message: String },
    /// A style file breaks the schema. The rendered problems follow the first line.
    #[code("style-error-invalid")]
    #[text("style `{style}` is invalid:\n{rendered}")]
    Invalid { style: String, rendered: String },
    /// `extends` forms a cycle. The chain lists the styles in order.
    #[code("style-error-cycle")]
    #[text("style extends cycle: {chain}")]
    Cycle { chain: String },
    /// Two active styles declare a conflict. The later style wins.
    #[code("style-conflict")]
    #[notice("style `{loser}` conflicts with `{winner}`; `{winner}` wins")]
    Conflict { loser: String, winner: String },
    /// Two active styles declare a conflict and set different values on these keys.
    #[code("style-conflict-on-keys")]
    #[text("style `{loser}` conflicts with `{winner}`; `{winner}` wins on {keys}")]
    ConflictOnKeys {
        loser: String,
        winner: String,
        keys: String,
    },
    /// A style file is larger than the style size cap. The code is the config catalog's.
    #[code("style-error-too-large")]
    #[text("[{code}] style `{style}` is larger than {cap} bytes")]
    TooLarge {
        code: String,
        style: String,
        #[count]
        cap: u64,
    },
    /// Reading a style file failed.
    #[code("style-error-io")]
    #[text("reading style `{style}`: {message}")]
    Io { style: String, message: String },
    /// A house style was found and a bundled preset matches it.
    #[code("style-detect-suggest-preset")]
    #[text("{style}: suggest the \"{preset}\" preset ({file}:{line})")]
    SuggestPreset {
        style: String,
        preset: String,
        #[display_path]
        file: String,
        #[count]
        line: usize,
    },
    /// A house style was found and no bundled preset matches it.
    #[code("style-detect-no-preset")]
    #[text("{style}: no bundled preset ({file}:{line})")]
    NoPreset {
        style: String,
        #[display_path]
        file: String,
        #[count]
        line: usize,
    },
    /// House-style detection skipped a file over the size limit.
    #[code("style-detect-file-too-large")]
    #[notice("{label} is over the 1 MiB style detection limit; house-style detection skipped it.")]
    DetectFileTooLarge {
        #[display_path]
        label: String,
    },
    /// The parser's message for a style file that is not valid TOML.
    #[code("style-toml-invalid")]
    #[text("not valid TOML: {reason}")]
    TomlInvalid { reason: String },
    /// The label under the position where the TOML parser stopped.
    #[code("style-toml-label")]
    #[text("the parser stopped here")]
    TomlLabel,
    /// A `max_per_100_words` budget below zero.
    #[code("style-budget-negative")]
    #[text("max_per_100_words must be at least 0, found {found}")]
    BudgetNegative { found: String },
    /// A `max_per_100_words` budget that is neither a number nor `none`.
    #[code("style-budget-invalid")]
    #[text("max_per_100_words must be a number or \"none\", found \"{found}\"")]
    BudgetInvalid { found: String },
}

pub(crate) fn not_found(name: &str) -> StyleMessage {
    StyleMessage::NotFound {
        name: name.to_owned(),
    }
}

pub(crate) fn invalid_name(name: &str) -> StyleMessage {
    StyleMessage::InvalidName {
        name: name.to_owned(),
    }
}

pub(crate) fn parse(style: &str, message: &str) -> StyleMessage {
    StyleMessage::Parse {
        style: style.to_owned(),
        message: message.to_owned(),
    }
}

pub(crate) fn invalid(style: &str, rendered: &str) -> StyleMessage {
    StyleMessage::Invalid {
        style: style.to_owned(),
        rendered: rendered.to_owned(),
    }
}

pub(crate) fn cycle(chain: &[String]) -> StyleMessage {
    StyleMessage::Cycle {
        chain: chain.join(" -> "),
    }
}

pub(crate) fn conflict(loser: &str, winner: &str) -> StyleMessage {
    StyleMessage::Conflict {
        loser: loser.to_owned(),
        winner: winner.to_owned(),
    }
}

/// The conflict warning: it names the keys when both styles set different values on some.
pub(crate) fn conflict_warning(loser: &str, winner: &str, keys: &[String]) -> StyleMessage {
    if keys.is_empty() {
        conflict(loser, winner)
    } else {
        StyleMessage::ConflictOnKeys {
            loser: loser.to_owned(),
            winner: winner.to_owned(),
            keys: keys.join(", "),
        }
    }
}

pub(crate) fn too_large(style: &str, cap: u64) -> StyleMessage {
    StyleMessage::TooLarge {
        code: Code::StyleTooLarge.as_str().to_owned(),
        style: style.to_owned(),
        cap,
    }
}

pub(crate) fn io(style: &str, message: &str) -> StyleMessage {
    StyleMessage::Io {
        style: style.to_owned(),
        message: message.to_owned(),
    }
}

/// The entries of the styles catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    StyleMessage::entries()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use slopvac_messages::Kind;

    use super::*;

    fn s(text: &str) -> String {
        text.to_owned()
    }

    #[rstest]
    #[case::not_found(StyleMessage::NotFound { name: s("calm") }, "unknown style `calm`")]
    #[case::invalid_name(
        StyleMessage::InvalidName { name: s("../x") },
        "invalid style name `../x`"
    )]
    #[case::parse(
        StyleMessage::Parse { style: s("calm"), message: s("bad") },
        "style `calm`: bad"
    )]
    #[case::invalid(
        StyleMessage::Invalid { style: s("calm"), rendered: s("line 1") },
        "style `calm` is invalid:\nline 1"
    )]
    #[case::cycle(StyleMessage::Cycle { chain: s("a -> b -> a") }, "style extends cycle: a -> b -> a")]
    #[case::conflict(
        StyleMessage::Conflict { loser: s("a"), winner: s("b") },
        "style `a` conflicts with `b`; `b` wins"
    )]
    #[case::conflict_on_keys(
        StyleMessage::ConflictOnKeys { loser: s("a"), winner: s("b"), keys: s("x, y") },
        "style `a` conflicts with `b`; `b` wins on x, y"
    )]
    #[case::too_large(
        StyleMessage::TooLarge { code: s("style-too-large"), style: s("calm"), cap: 1_048_576 },
        "[style-too-large] style `calm` is larger than 1048576 bytes"
    )]
    #[case::io(
        StyleMessage::Io { style: s("calm"), message: s("denied") },
        "reading style `calm`: denied"
    )]
    #[case::suggest_preset(
        StyleMessage::SuggestPreset {
            style: s("Google"),
            preset: s("google"),
            file: s("STYLE.md"),
            line: 3,
        },
        "Google: suggest the \"google\" preset (STYLE.md:3)"
    )]
    #[case::no_preset(
        StyleMessage::NoPreset { style: s("Acme"), file: s(".vale.ini"), line: 1 },
        "Acme: no bundled preset (.vale.ini:1)"
    )]
    #[case::detect_too_large(
        StyleMessage::DetectFileTooLarge { label: s("STYLE.md") },
        "STYLE.md is over the 1 MiB style detection limit; house-style detection skipped it."
    )]
    #[case::toml_invalid(
        StyleMessage::TomlInvalid { reason: s("expected `=`") },
        "not valid TOML: expected `=`"
    )]
    #[case::toml_label(StyleMessage::TomlLabel, "the parser stopped here")]
    #[case::budget_negative(
        StyleMessage::BudgetNegative { found: s("-1") },
        "max_per_100_words must be at least 0, found -1"
    )]
    #[case::budget_invalid(
        StyleMessage::BudgetInvalid { found: s("lots") },
        "max_per_100_words must be a number or \"none\", found \"lots\""
    )]
    fn each_message_renders_its_published_text(
        #[case] message: StyleMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[test]
    fn the_catalog_registers_each_code_with_its_kind() {
        let entries: Vec<_> = message_entries().iter().map(|e| (e.code, e.kind)).collect();
        assert_eq!(
            entries,
            [
                ("style-error-not-found", Kind::Text),
                ("style-error-invalid-name", Kind::Text),
                ("style-error-parse", Kind::Text),
                ("style-error-invalid", Kind::Text),
                ("style-error-cycle", Kind::Text),
                ("style-conflict", Kind::Notice),
                ("style-conflict-on-keys", Kind::Text),
                ("style-error-too-large", Kind::Text),
                ("style-error-io", Kind::Text),
                ("style-detect-suggest-preset", Kind::Text),
                ("style-detect-no-preset", Kind::Text),
                ("style-detect-file-too-large", Kind::Notice),
                ("style-toml-invalid", Kind::Text),
                ("style-toml-label", Kind::Text),
                ("style-budget-negative", Kind::Text),
                ("style-budget-invalid", Kind::Text),
            ]
        );
    }
}
