//! The messages `slopvac-terms` reports: error text, validation sentences, command output
//! and help.
//!
//! Three catalogs hold them. [`TermsMessage`] renders each [`crate::TermsError`] variant.
//! [`TermsText`] holds the sentences with parameters: what is wrong with an entry, and what
//! `slopvac terms` prints. [`TermsPhrase`] holds fixed phrases, which callers read as
//! `&'static str` through [`TermsPhrase::as_static`]. The binary registers the three through
//! [`message_entries`], [`text_entries`] and [`phrase_entries`].

use slopvac_messages::{Catalog, Entry};

/// The text of each [`crate::TermsError`] variant.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum TermsMessage {
    /// A terms or blocklist file could not be read, or an output could not be written.
    #[code("terms-error-io")]
    #[text("cannot read {path}: {message}")]
    Io {
        #[display_path]
        path: String,
        message: String,
    },
    /// A terms or blocklist file is larger than the terms file cap.
    #[code("terms-file-too-large")]
    #[message("[terms-file-too-large] {path} is larger than {cap} bytes")]
    FileTooLarge {
        #[display_path]
        path: String,
        #[count]
        cap: u64,
    },
    /// A file is larger than its size cap. The code names the cap that applies.
    #[code("terms-error-too-large")]
    #[text("[{code}] {path} is larger than {cap} bytes")]
    TooLarge {
        code: String,
        #[display_path]
        path: String,
        #[count]
        cap: u64,
    },
    /// A file is not valid for its format.
    #[code("terms-error-parse")]
    #[text("cannot parse {path}: {message}")]
    Parse {
        #[display_path]
        path: String,
        message: String,
    },
    /// A blocklist has no `schema_version`, as a 2.x blocklist.
    #[code("terms-error-missing-schema-version")]
    #[text(
        "{path}: missing `schema_version`; a 3.0 blocklist sets `schema_version = 3`. Run `slopvac migrate` to move a 2.x blocklist to 3.0"
    )]
    MissingSchemaVersion {
        #[display_path]
        path: String,
    },
    /// A file declares a `schema_version` that is not 3. The value shows as JSON text.
    #[code("terms-error-schema-version")]
    #[text(
        "{path}: schema_version {found} is not supported, expected the integer 3. Run `slopvac migrate` to move an older file to 3.0"
    )]
    SchemaVersion {
        #[display_path]
        path: String,
        found: String,
    },
    /// A file has a key the schema does not define.
    #[code("terms-error-unknown-key")]
    #[text("{path}: unknown key `{key}`")]
    UnknownKey {
        #[display_path]
        path: String,
        key: String,
    },
    /// A term or blocklist entry is invalid. The message says which entry and why.
    #[code("terms-error-entry")]
    #[text("{path}: {message}")]
    Entry {
        #[display_path]
        path: String,
        message: String,
    },
}

/// Sentences with parameters: entry problems, import problems and command output.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum TermsText {
    /// A terms entry has an empty term.
    #[code("terms-entry-term-empty")]
    #[text("term is empty")]
    TermEmpty,
    /// A terms entry at a position has an empty term.
    #[code("terms-entry-term-empty-at")]
    #[text("term {number} is empty")]
    TermEmptyAt {
        #[count]
        number: usize,
    },
    /// A term sets both casing modes.
    #[code("terms-entry-conflicting-casing")]
    #[text("term `{term}` sets both ignore_case and case_sensitive")]
    ConflictingCasing { term: String },
    /// A term has a kind that is not a kebab-case label. The kind shows quoted.
    #[code("terms-entry-bad-kind")]
    #[text("term `{term}` has kind {kind}; use a kebab-case label such as `product`")]
    BadKind { term: String, kind: String },
    /// A term has an empty variant.
    #[code("terms-entry-empty-variant")]
    #[text("term `{term}` has an empty variant")]
    EmptyVariant { term: String },
    /// A term lists a variant twice.
    #[code("terms-entry-repeated-variant")]
    #[text("term `{term}` lists variant `{variant}` twice")]
    RepeatedVariant { term: String, variant: String },
    /// Two terms have the same canonical spelling.
    #[code("terms-entry-duplicate-term")]
    #[text("duplicate term `{term}`")]
    DuplicateTerm { term: String },
    /// The term to add is already in the file.
    #[code("terms-entry-term-exists")]
    #[text("term `{term}` already exists")]
    TermExists { term: String },
    /// A terms file has no `terms` array.
    #[code("terms-file-missing-array")]
    #[text("missing `terms` array; use `terms = []` for an empty list")]
    MissingTermsArray,
    /// A blocklist file has a suffix that names no known format.
    #[code("terms-blocklist-bad-suffix")]
    #[text("unrecognized suffix `{suffix}`; use .toml, .json, .yml or .yaml")]
    BadSuffix { suffix: String },
    /// A blocklist has no `entries` array. The other keys it defines show as a list.
    #[code("terms-blocklist-missing-entries")]
    #[text("no `entries` array; it defines {named}. Each word is an `[[entries]]` table")]
    MissingEntries { named: String },
    /// A blocklist entry has no word.
    #[code("terms-blocklist-entry-no-word")]
    #[text("entry {number} has no `word`")]
    EntryNoWord {
        #[count]
        number: usize,
    },
    /// A blocklist entry has a part of speech that is not recognised. The value shows quoted.
    #[code("terms-blocklist-entry-unknown-pos")]
    #[text(
        "entry `{word}` has an unrecognized `pos` {pos}; use noun, verb, adjective, adverb, preposition, conjunction, pronoun or article"
    )]
    EntryUnknownPos { word: String, pos: String },
    /// A blocklist entry has no reason.
    #[code("terms-blocklist-entry-no-reason")]
    #[text("entry `{word}` ({pos}) has no `reason`; every refused word needs one")]
    EntryNoReason { word: String, pos: String },
    /// Two blocklist entries name the same word and part of speech.
    #[code("terms-blocklist-entry-duplicate")]
    #[text("duplicate entry `{word}` ({pos}) at entries {first} and {number}")]
    EntryDuplicate {
        word: String,
        pos: String,
        #[count]
        first: usize,
        #[count]
        number: usize,
    },
    /// The import source is a cspell YAML config.
    #[code("terms-import-cspell-yaml")]
    #[text("cspell YAML configs are not supported; convert the file to JSON")]
    CspellYaml,
    /// A codespell `ignore-words-list` is a TOML array.
    #[code("terms-import-words-array")]
    #[text("ignore-words-list: TOML arrays are not supported; use a comma-separated string")]
    IgnoreWordsArray,
    /// A codespell `ignore-words-list` has an unbalanced quote.
    #[code("terms-import-words-quote")]
    #[text("ignore-words-list: unbalanced quote")]
    IgnoreWordsQuote,
    /// `terms add` added a term. The case mode follows in parentheses.
    #[code("terms-added-with-mode")]
    #[text("added {term} ({mode})")]
    AddedWithMode { term: String, mode: String },
    /// `terms suggest --accept` added a term.
    #[code("terms-added")]
    #[text("added {term}")]
    Added { term: String },
    /// `terms remove` removed a term.
    #[code("terms-removed")]
    #[text("removed {term}")]
    Removed { term: String },
    /// `terms remove` found no such term.
    #[code("terms-remove-missing")]
    #[text("no term {term} in {file}")]
    RemoveMissing {
        term: String,
        #[display_path]
        file: String,
    },
    /// `terms check` found a wrong-case term.
    #[code("terms-check-finding")]
    #[text("{file}:{line}:{column}: {found} -> {expected}")]
    CheckFinding {
        file: String,
        #[count]
        line: usize,
        #[count]
        column: usize,
        found: String,
        expected: String,
    },
    /// `terms suggest --accept` named a term that is not a suggestion.
    #[code("terms-accept-unknown")]
    #[text("`{term}` is not a suggestion; run without --accept to see them")]
    AcceptUnknown { term: String },
    /// The summary line of `terms import`.
    #[code("terms-import-summary")]
    #[text("imported {added} terms ({existing} already present, {skipped} patterns skipped)")]
    ImportSummary {
        #[count]
        added: usize,
        #[count]
        existing: usize,
        #[count]
        skipped: usize,
    },
    /// A pattern that `terms import` skipped.
    #[code("terms-import-skipped")]
    #[text("skipped pattern: {pattern}")]
    ImportSkipped { pattern: String },
}

/// Fixed phrases: problem descriptions, case modes, the word for an empty list and command
/// help. Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum TermsPhrase {
    /// An entry has no word.
    #[code("terms-problem-missing-word")]
    #[text("has no word")]
    ProblemMissingWord,
    /// An entry has an unrecognised part of speech.
    #[code("terms-problem-unknown-pos")]
    #[text("has an unrecognized part of speech")]
    ProblemUnknownPos,
    /// An entry has no reason.
    #[code("terms-problem-missing-reason")]
    #[text("has no reason")]
    ProblemMissingReason,
    /// An entry repeats another.
    #[code("terms-problem-duplicate")]
    #[text("is a duplicate")]
    ProblemDuplicate,
    /// A term sets both casing modes.
    #[code("terms-problem-conflicting-casing")]
    #[text("sets both casing modes")]
    ProblemConflictingCasing,
    /// A term has a kind that is not a kebab-case label.
    #[code("terms-problem-bad-kind")]
    #[text("has a kind that is not a kebab-case label")]
    ProblemBadKind,
    /// A term has an empty or repeated variant.
    #[code("terms-problem-bad-variant")]
    #[text("has an empty or repeated variant")]
    ProblemBadVariant,
    /// Any other problem with an entry.
    #[code("terms-problem-other")]
    #[text("is not valid")]
    ProblemOther,
    /// What a blocklist without an `entries` array defines.
    #[code("terms-defines-nothing")]
    #[text("nothing")]
    DefinesNothing,
    /// The case mode of a term that ignores case.
    #[code("terms-mode-ignore-case")]
    #[text("ignoring case")]
    ModeIgnoreCase,
    /// The case mode of a term that matches the exact spelling only.
    #[code("terms-mode-exact")]
    #[text("exact spelling only")]
    ModeExact,
    /// The case mode of a term that flags any other casing.
    #[code("terms-mode-flags-wrong-case")]
    #[text("flags wrong case")]
    ModeFlagsWrongCase,
    /// Command help for `terms`.
    #[code("terms-about")]
    #[text("Manage project terms")]
    About,
    /// Command help for `terms list`.
    #[code("terms-list-about")]
    #[text("List terms")]
    ListAbout,
    /// Command help for `terms search`.
    #[code("terms-search-about")]
    #[text("Find terms by substring, ignoring case")]
    SearchAbout,
    /// Command help for `terms add`.
    #[code("terms-add-about")]
    #[text("Add a term; lower-case terms default to ignoring case")]
    AddAbout,
    /// Command help for `terms remove`.
    #[code("terms-remove-about")]
    #[text("Remove a term")]
    RemoveAbout,
    /// Command help for `terms check`.
    #[code("terms-check-about")]
    #[text("Report wrong-case terms in files; exit 1 when any are found")]
    CheckAbout,
    /// Command help for `terms suggest`.
    #[code("terms-suggest-about")]
    #[text(
        "List inferred proper nouns and acronyms that are not terms yet; nothing is written unless you name a term with --accept"
    )]
    SuggestAbout,
    /// Command help for `terms import`.
    #[code("terms-import-about")]
    #[text(
        "Import words from another tool: a Vale accept.txt, a cspell JSON config (YAML configs are not supported) or a codespell ignore list"
    )]
    ImportAbout,
    /// Help for the `--files` option.
    #[code("terms-files-help")]
    #[text("Terms file (default: .slopvac/terms.toml)")]
    FilesHelp,
    /// Help for the `--builtin` flag.
    #[code("terms-builtin-help")]
    #[text("Include the built-in list")]
    BuiltinHelp,
    /// Help for the `--case-sensitive` flag.
    #[code("terms-case-sensitive-help")]
    #[text("Match only the exact spelling; other casings are different words")]
    CaseSensitiveHelp,
    /// Help for the `--min` option.
    #[code("terms-min-help")]
    #[text("Only candidates seen at least N times")]
    MinHelp,
    /// Help for the `--accept` option.
    #[code("terms-accept-help")]
    #[text("Add this suggested spelling to the terms file")]
    AcceptHelp,
}

/// The entries of `TermsMessage`: the text of each error variant.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    TermsMessage::entries()
}

/// The entries of `TermsText`: sentences with parameters.
#[must_use]
pub fn text_entries() -> &'static [Entry] {
    TermsText::entries()
}

/// The entries of `TermsPhrase`: fixed phrases.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    TermsPhrase::entries()
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
    #[case::io(
        TermsMessage::Io { path: s("t.toml"), message: s("denied") },
        "cannot read t.toml: denied"
    )]
    #[case::too_large(
        TermsMessage::TooLarge { code: s("terms-file-too-large"), path: s("t.toml"), cap: 1_048_576 },
        "[terms-file-too-large] t.toml is larger than 1048576 bytes"
    )]
    #[case::file_too_large(
        TermsMessage::FileTooLarge { path: s("t.toml"), cap: 1_048_576 },
        "[terms-file-too-large] t.toml is larger than 1048576 bytes"
    )]
    #[case::parse(
        TermsMessage::Parse { path: s("t.toml"), message: s("bad") },
        "cannot parse t.toml: bad"
    )]
    #[case::missing_version(
        TermsMessage::MissingSchemaVersion { path: s("b.toml") },
        "b.toml: missing `schema_version`; a 3.0 blocklist sets `schema_version = 3`. Run `slopvac migrate` to move a 2.x blocklist to 3.0"
    )]
    #[case::schema_version(
        TermsMessage::SchemaVersion { path: s("t.toml"), found: s("2") },
        "t.toml: schema_version 2 is not supported, expected the integer 3. Run `slopvac migrate` to move an older file to 3.0"
    )]
    #[case::unknown_key(
        TermsMessage::UnknownKey { path: s("t.toml"), key: s("oops") },
        "t.toml: unknown key `oops`"
    )]
    #[case::entry(
        TermsMessage::Entry { path: s("t.toml"), message: s("term is empty") },
        "t.toml: term is empty"
    )]
    fn each_error_message_renders_its_published_text(
        #[case] message: TermsMessage,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[rstest]
    #[case::term_empty(TermsText::TermEmpty, "term is empty")]
    #[case::term_empty_at(TermsText::TermEmptyAt { number: 3 }, "term 3 is empty")]
    #[case::conflicting_casing(
        TermsText::ConflictingCasing { term: s("Go") },
        "term `Go` sets both ignore_case and case_sensitive"
    )]
    #[case::bad_kind(
        TermsText::BadKind { term: s("Go"), kind: s("\"Lang\"") },
        "term `Go` has kind \"Lang\"; use a kebab-case label such as `product`"
    )]
    #[case::empty_variant(
        TermsText::EmptyVariant { term: s("Go") },
        "term `Go` has an empty variant"
    )]
    #[case::repeated_variant(
        TermsText::RepeatedVariant { term: s("Go"), variant: s("golang") },
        "term `Go` lists variant `golang` twice"
    )]
    #[case::duplicate_term(TermsText::DuplicateTerm { term: s("Go") }, "duplicate term `Go`")]
    #[case::term_exists(TermsText::TermExists { term: s("Go") }, "term `Go` already exists")]
    #[case::missing_array(
        TermsText::MissingTermsArray,
        "missing `terms` array; use `terms = []` for an empty list"
    )]
    #[case::bad_suffix(
        TermsText::BadSuffix { suffix: s("txt") },
        "unrecognized suffix `txt`; use .toml, .json, .yml or .yaml"
    )]
    #[case::missing_entries(
        TermsText::MissingEntries { named: s("`words`") },
        "no `entries` array; it defines `words`. Each word is an `[[entries]]` table"
    )]
    #[case::entry_no_word(TermsText::EntryNoWord { number: 2 }, "entry 2 has no `word`")]
    #[case::entry_unknown_pos(
        TermsText::EntryUnknownPos { word: s("fast"), pos: s("\"adj\"") },
        "entry `fast` has an unrecognized `pos` \"adj\"; use noun, verb, adjective, adverb, preposition, conjunction, pronoun or article"
    )]
    #[case::entry_no_reason(
        TermsText::EntryNoReason { word: s("fast"), pos: s("Adjective") },
        "entry `fast` (Adjective) has no `reason`; every refused word needs one"
    )]
    #[case::entry_duplicate(
        TermsText::EntryDuplicate { word: s("fast"), pos: s("Adjective"), first: 1, number: 4 },
        "duplicate entry `fast` (Adjective) at entries 1 and 4"
    )]
    #[case::cspell_yaml(
        TermsText::CspellYaml,
        "cspell YAML configs are not supported; convert the file to JSON"
    )]
    #[case::words_array(
        TermsText::IgnoreWordsArray,
        "ignore-words-list: TOML arrays are not supported; use a comma-separated string"
    )]
    #[case::words_quote(TermsText::IgnoreWordsQuote, "ignore-words-list: unbalanced quote")]
    #[case::added_with_mode(
        TermsText::AddedWithMode { term: s("Go"), mode: s("ignoring case") },
        "added Go (ignoring case)"
    )]
    #[case::added(TermsText::Added { term: s("Go") }, "added Go")]
    #[case::removed(TermsText::Removed { term: s("Go") }, "removed Go")]
    #[case::remove_missing(
        TermsText::RemoveMissing { term: s("Go"), file: s(".slopvac/terms.toml") },
        "no term Go in .slopvac/terms.toml"
    )]
    #[case::check_finding(
        TermsText::CheckFinding {
            file: s("a.md"), line: 3, column: 9, found: s("github"), expected: s("GitHub"),
        },
        "a.md:3:9: github -> GitHub"
    )]
    #[case::accept_unknown(
        TermsText::AcceptUnknown { term: s("Go") },
        "`Go` is not a suggestion; run without --accept to see them"
    )]
    #[case::import_summary(
        TermsText::ImportSummary { added: 5, existing: 2, skipped: 1 },
        "imported 5 terms (2 already present, 1 patterns skipped)"
    )]
    #[case::import_skipped(
        TermsText::ImportSkipped { pattern: s("foo*") },
        "skipped pattern: foo*"
    )]
    fn each_text_message_renders_its_published_text(
        #[case] message: TermsText,
        #[case] expected: &str,
    ) {
        assert_eq!(message.to_string(), expected);
    }

    #[rstest]
    #[case::missing_word(TermsPhrase::ProblemMissingWord, "has no word")]
    #[case::unknown_pos(TermsPhrase::ProblemUnknownPos, "has an unrecognized part of speech")]
    #[case::missing_reason(TermsPhrase::ProblemMissingReason, "has no reason")]
    #[case::duplicate(TermsPhrase::ProblemDuplicate, "is a duplicate")]
    #[case::conflicting_casing(TermsPhrase::ProblemConflictingCasing, "sets both casing modes")]
    #[case::bad_kind(
        TermsPhrase::ProblemBadKind,
        "has a kind that is not a kebab-case label"
    )]
    #[case::bad_variant(TermsPhrase::ProblemBadVariant, "has an empty or repeated variant")]
    #[case::other(TermsPhrase::ProblemOther, "is not valid")]
    #[case::defines_nothing(TermsPhrase::DefinesNothing, "nothing")]
    #[case::mode_ignore_case(TermsPhrase::ModeIgnoreCase, "ignoring case")]
    #[case::mode_exact(TermsPhrase::ModeExact, "exact spelling only")]
    #[case::mode_flags(TermsPhrase::ModeFlagsWrongCase, "flags wrong case")]
    #[case::about(TermsPhrase::About, "Manage project terms")]
    #[case::list_about(TermsPhrase::ListAbout, "List terms")]
    #[case::search_about(TermsPhrase::SearchAbout, "Find terms by substring, ignoring case")]
    #[case::add_about(
        TermsPhrase::AddAbout,
        "Add a term; lower-case terms default to ignoring case"
    )]
    #[case::remove_about(TermsPhrase::RemoveAbout, "Remove a term")]
    #[case::check_about(
        TermsPhrase::CheckAbout,
        "Report wrong-case terms in files; exit 1 when any are found"
    )]
    #[case::suggest_about(
        TermsPhrase::SuggestAbout,
        "List inferred proper nouns and acronyms that are not terms yet; nothing is written unless you name a term with --accept"
    )]
    #[case::import_about(
        TermsPhrase::ImportAbout,
        "Import words from another tool: a Vale accept.txt, a cspell JSON config (YAML configs are not supported) or a codespell ignore list"
    )]
    #[case::files_help(TermsPhrase::FilesHelp, "Terms file (default: .slopvac/terms.toml)")]
    #[case::builtin_help(TermsPhrase::BuiltinHelp, "Include the built-in list")]
    #[case::case_sensitive_help(
        TermsPhrase::CaseSensitiveHelp,
        "Match only the exact spelling; other casings are different words"
    )]
    #[case::min_help(TermsPhrase::MinHelp, "Only candidates seen at least N times")]
    #[case::accept_help(
        TermsPhrase::AcceptHelp,
        "Add this suggested spelling to the terms file"
    )]
    fn each_phrase_reads_as_its_published_text(
        #[case] phrase: TermsPhrase,
        #[case] expected: &str,
    ) {
        assert_eq!(phrase.as_static(), expected);
        assert_eq!(phrase.to_string(), expected);
    }

    #[test]
    fn the_catalogs_register_their_codes_as_text() {
        let all: Vec<_> = [message_entries(), text_entries(), phrase_entries()]
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(all.len(), 59);
        let kinds: Vec<_> = all
            .iter()
            .filter(|e| e.kind != Kind::Text)
            .map(|e| (e.code, e.kind))
            .collect();
        assert_eq!(kinds, [("terms-file-too-large", Kind::Error)]);
        assert!(all.iter().all(|e| e.code.starts_with("terms-")));
    }
}
