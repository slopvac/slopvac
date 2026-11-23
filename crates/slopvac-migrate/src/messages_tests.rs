//! Every message, rendered with sample parameters, beside the text that 3.0 builds published
//! before the registry. A change to any pair changes what users read.

use super::*;

fn s(text: &str) -> String {
    text.to_owned()
}

const POINTER: &str = "see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml";

#[test]
fn a_blocklist_over_the_terms_cap_reads_with_the_terms_wording_and_code() {
    let error = Error::TermsFileTooLarge {
        path: s("t.toml"),
        cap: 1_048_576,
    };
    assert_eq!(error.code(), "terms-file-too-large");
    assert_eq!(
        error.to_string(),
        "[terms-file-too-large] t.toml is larger than 1048576 bytes"
    );
}

#[test]
fn every_code_keeps_its_published_text_and_is_registered() {
    let messages: Vec<(MigrateMessage, String)> = vec![
        (
            MigrateMessage::LegacyDropped {
                records: s("a\n  b"),
            },
            s("a\n  b"),
        ),
        (
            MigrateMessage::AlreadyMigrated {
                file: s("slopvac.toml"),
            },
            s("slopvac.toml: already migrated (schema_version is 3 or higher)"),
        ),
        (MigrateMessage::ConfigConflict { detail: s("d") }, s("d")),
        (
            MigrateMessage::UnsafePath { detail: s("d") },
            s("unsafe: d"),
        ),
        (
            MigrateMessage::FileChanged { detail: s("d") },
            s("changed: d"),
        ),
        (
            MigrateMessage::OverrideOverlap {
                file: s("slopvac.toml"),
                line: s("4"),
                block: s("1"),
                reason: s("unsupported pattern \"*.md\""),
            },
            s(
                "slopvac.toml:4: `overrides[1]`: cannot decide how this block overlaps the others: unsupported pattern \"*.md\". --preserve-behavior accepts one pattern per block: an anchored path such as `docs/api` or a tree such as `docs/**`",
            ),
        ),
        (
            MigrateMessage::RenameTable { detail: s("d") },
            s("rename table: d"),
        ),
        (MigrateMessage::LintFailed { detail: s("d") }, s("lint: d")),
        (
            MigrateMessage::OutputInvalid {
                file: s("slopvac.toml"),
                reason: s("r"),
            },
            s("slopvac.toml: the migrated config is refused, so nothing is written: r"),
        ),
        (
            MigrateMessage::RunLimit {
                path: s("docs"),
                limit: 5000,
                what: s("files"),
            },
            s("docs: run limit reached: more than 5000 files in one run"),
        ),
        (
            MigrateMessage::NotUtf8 { path: s("a.md") },
            s("a.md: not valid UTF-8"),
        ),
        (
            MigrateMessage::IoFailed {
                path: s("a.md"),
                reason: s("denied"),
            },
            s("a.md: denied"),
        ),
        (
            MigrateMessage::RuleRemoved { setting: s("x") },
            format!("migrate-rule-removed: x; {POINTER}"),
        ),
        (
            MigrateMessage::CategoryRemoved { setting: s("x") },
            format!("migrate-category-removed: x; {POINTER}"),
        ),
    ];
    let registered = error_entries();
    for (message, text) in &messages {
        assert_eq!(&message.to_string(), text);
        assert!(registered.iter().any(|e| e.code == message.code()));
    }
    assert_eq!(registered.len(), messages.len());
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table of every published text, which reads best in one place"
)]
fn every_sentence_keeps_its_published_text_and_is_registered() {
    let texts: Vec<(MigrateText, &str)> = vec![
        (
            MigrateText::ParseFailed {
                file: s("p.toml"),
                message: s("line 2: bad"),
            },
            "p.toml: line 2: bad",
        ),
        (
            MigrateText::InvalidType {
                file: s("p.toml"),
                line: s("3"),
                message: s("m"),
            },
            "p.toml:3: m",
        ),
        (
            MigrateText::PathTooLarge {
                path: s("a.md"),
                cap: 64,
            },
            "a.md: larger than 64 bytes",
        ),
        (
            MigrateText::PathReason {
                path: s("a.md"),
                reason: s("is a symbolic link"),
            },
            "a.md: is a symbolic link",
        ),
        (
            MigrateText::DroppedAt {
                file: s("f"),
                line: s("2"),
                key: s("k"),
                reason: s("r"),
            },
            "f:2: `k`: r",
        ),
        (
            MigrateText::DroppedSuppressed { count: 3 },
            "3 more blocking settings suppressed",
        ),
        (
            MigrateText::NestingTooDeep { limit: 32 },
            "tables nest more than 32 levels deep",
        ),
        (
            MigrateText::SuppressedNotes { count: 3 },
            "3 more notes suppressed",
        ),
        (
            MigrateText::SuppressedWarnings { count: 3 },
            "3 more warnings suppressed",
        ),
        (
            MigrateText::ConfigSources {
                directory: s("docs"),
            },
            "docs: more than one 2.x config source (slopvac.toml, .slopvac.toml, [tool.slopvac]); 2.x read only one of them. Remove the ones that are unused, then rerun",
        ),
        (
            MigrateText::NotCheckedSymlink { path: s("l") },
            "l: not checked: symbolic link (links are not followed)",
        ),
        (
            MigrateText::NotCheckedKind {
                path: s("a.rst"),
                kind: s("Rst"),
            },
            "a.rst: not checked: migrate reads Markdown and MDX only, not Rst files",
        ),
        (
            MigrateText::NotCheckedIrregular { path: s("p") },
            "p: not checked: not a regular file",
        ),
        (
            MigrateText::OverlapManyPatterns {
                count: 2,
                patterns: s("[\"a\", \"b\"]"),
            },
            "a block with 2 patterns: [\"a\", \"b\"]; use one pattern per block",
        ),
        (
            MigrateText::OverlapUnsupported {
                pattern: s("\"*\""),
            },
            "unsupported pattern \"*\"",
        ),
        (
            MigrateText::LintNotFullyChecked {
                label: s("a.md"),
                code: s("c"),
                message: s("m"),
            },
            "a.md: not fully checked: c: m",
        ),
        (
            MigrateText::LintDocumentFailed {
                label: s("a.md"),
                detail: s("d"),
            },
            "a.md: d",
        ),
        (
            MigrateText::RenameRemovedNotArray { key: s("rules") },
            "removed.rules: expected an array",
        ),
        (
            MigrateText::RenameRemovedNotStrings { key: s("rules") },
            "removed.rules: expected strings",
        ),
        (
            MigrateText::RenameMissingTable { key: s("rules") },
            "missing [rules] table",
        ),
        (
            MigrateText::RenameNotString {
                key: s("rules"),
                name: s("a"),
            },
            "rules.a: expected a string",
        ),
        (
            MigrateText::DirectiveKey { id: s("a.b") },
            "slopvac-allow rule=a.b",
        ),
        (
            MigrateText::DirectiveInvalid {
                path: s("a.md"),
                line: s("5"),
            },
            "a.md:5: invalid 2.x directive, left as is",
        ),
        (
            MigrateText::DirectiveMdx {
                path: s("a.mdx"),
                line: s("5"),
            },
            "a.mdx:5: MDX is not rewritten; rename or remove this slopvac-allow directive by hand",
        ),
        (
            MigrateText::NoteBeforeApproximate {
                file: s("slopvac.toml"),
                line: s("4"),
                reason: s("r"),
            },
            "slopvac.toml:4: the before side for this config is approximate, because its 2.x settings could not be reproduced: r",
        ),
        (
            MigrateText::NoteRenamedDotfile { label: s("l") },
            "l: renamed to slopvac.toml",
        ),
        (
            MigrateText::NoteMovedPyproject { label: s("l") },
            "l: moved [tool.slopvac] to slopvac.toml",
        ),
        (
            MigrateText::ParseAtLine {
                line: s("2"),
                message: s("m"),
            },
            "line 2: m",
        ),
        (
            MigrateText::NoteRenamedCategory {
                from: s("a"),
                to: s("b"),
            },
            "renamed category a to b",
        ),
        (
            MigrateText::NoteDroppedExtensionKey { key: s("x-k") },
            "dropped extension key x-k",
        ),
        (
            MigrateText::NoteMovedDefault { path: s("locale") },
            "moved locale.default to locale",
        ),
        (
            MigrateText::NoteMovedAllow { path: s("locale") },
            "moved locale.allow to terms.allow",
        ),
        (
            MigrateText::NoteMovedBlocklistPath {
                path: s("vocabulary"),
            },
            "moved vocabulary.path to terms.blocklists",
        ),
        (
            MigrateText::NoteDeletedVale { path: s("vale") },
            "deleted vale (Vale is removed in 3.0)",
        ),
        (
            MigrateText::BlocklistOutside {
                file: s("f"),
                line: s("2"),
                key: s("k"),
            },
            "f:2: `k`: the blocklist is outside the project root, so migrate does not rewrite it. Move it into the project, or give it `schema_version = 3` by hand",
        ),
        (
            MigrateText::BlocklistSymlink {
                file: s("f"),
                line: s("2"),
                key: s("k"),
            },
            "f:2: `k`: the blocklist path crosses a symbolic link, so migrate cannot tell which file the project loads. Name the file without the link",
        ),
        (
            MigrateText::BlocklistBlocked {
                label: s("t.toml"),
                reason: s("file not found"),
            },
            "blocklist `t.toml`: file not found",
        ),
        (
            MigrateText::BlocklistFormatRefused {
                format: s("json"),
                what: s("has an unknown key"),
            },
            "3.0 refuses this .json blocklist (has an unknown key). migrate rewrites TOML blocklists only. Add `schema_version: 3` and long-form `pos` values by hand, or convert it to TOML",
        ),
        (
            MigrateText::BlocklistFileProblem { what: s("w") },
            "the file w",
        ),
        (
            MigrateText::BlocklistStillRefused { what: s("w") },
            "3.0 refuses the migrated blocklist: w",
        ),
        (
            MigrateText::NoteAddedBlocklistVersion { label: s("t.toml") },
            "t.toml: added schema_version = 3",
        ),
        (
            MigrateText::NotePosRewritten {
                label: s("t.toml"),
                line: s("4"),
                long: s("noun"),
            },
            "t.toml:4: pos rewritten to `noun`",
        ),
        (
            MigrateText::BlocklistEntryNumbered {
                number: s("2"),
                problem: s("p"),
            },
            "entry 2 p",
        ),
        (
            MigrateText::BlocklistEntry { problem: s("p") },
            "an entry p",
        ),
        (
            MigrateText::NoteDroppedJudgement { path: s("rules.a") },
            "dropped rules.a: a judgement rule takes its review level from the legacy style",
        ),
        (
            MigrateText::NoteDroppedIgnored { path: s("rules.a") },
            "dropped rules.a: 2.x ignored it for every file it applies to",
        ),
        (
            MigrateText::NoteAddedStyle {
                style: s("core:legacy-strict"),
                profile: s("strict"),
            },
            "added styles = [\"core:legacy-strict\"] to reproduce the 2.x strict profile",
        ),
        (
            MigrateText::NoteAddedVocabularyStyle {
                style: s("core:legacy-strict-vocabulary"),
                profile: s("strict"),
            },
            "added \"core:legacy-strict-vocabulary\" to styles: 2.x ran the vocabulary rules once a word blocklist was set, at the level of the strict profile",
        ),
        (
            MigrateText::ReasonBlocklistInOverride {
                at: s("overrides[0].vocabulary.path"),
                line: s("4"),
            },
            "`overrides[0].vocabulary.path` on line 4 sets a word blocklist, and the top level sets none. `--preserve-behavior` cannot keep the 2.x vocabulary levels for a blocklist that applies to some files only. A follow-up will support these layouts.",
        ),
        (
            MigrateText::ReasonBlocklistUnderSwitch {
                at: s("overrides[0].profile"),
                line: s("5"),
            },
            "`overrides[0].profile` on line 5 sets `profile` in an override, whatever its value, and the project sets a word blocklist. `--preserve-behavior` cannot keep the 2.x vocabulary levels in the files that override covers. A follow-up will support these layouts.",
        ),
        (
            MigrateText::ReasonOverrideNotTable {
                at: s("overrides[1]"),
                line: s("1"),
            },
            "`overrides[1]` on line 1 is not an override table. 2.x reads `overrides` as a list of tables, and the migration cannot rewrite the list without dropping that entry.",
        ),
        (
            MigrateText::NoteMergedSwitch {
                profile: s("relaxed"),
                at: s("overrides[1]"),
            },
            "merged the switch to the 2.x relaxed profile into overrides[1]",
        ),
        (
            MigrateText::NoteRestatedBlocks {
                at: s("overrides[1]"),
                blocks: s("overrides[0]"),
            },
            "restated in overrides[1] the categories and rules of overrides[0], which also match its files",
        ),
        (
            MigrateText::NoteMovedBlock {
                at: s("overrides[0]"),
                after: s("overrides[1]"),
                blocks: s("overrides[1]"),
            },
            "moved overrides[0] after overrides[1], which holds its files and switches them to another 2.x profile; it drops the settings that overrides[1] also sets, which won in 2.x",
        ),
        (
            MigrateText::NoteGuardedRuleEntry {
                count: 1,
                at: s("overrides[0]"),
            },
            "turned off 1 rule entry in overrides[0] that 2.x ignored for its files",
        ),
        (
            MigrateText::NoteGuardedRuleEntries {
                count: 2,
                at: s("overrides[0]"),
            },
            "turned off 2 rule entries in overrides[0] that 2.x ignored for its files",
        ),
        (
            MigrateText::NoteAddedGuardBlocks {
                blocks: s("1 override block"),
            },
            "added 1 override block after the others that turn off rule entries 2.x ignored",
        ),
        (
            MigrateText::OverrideBlocks { count: 3 },
            "3 override blocks",
        ),
        (
            MigrateText::NoteSetRuleLevel {
                at: s("rules.a"),
                level: s("error"),
            },
            "set rules.a to \"error\": the 2.x profile default replaces an entry 2.x ignored",
        ),
        (
            MigrateText::NoteSetRuleLevelApplied {
                at: s("overrides[0].rules.a"),
                level: s("off"),
            },
            "set overrides[0].rules.a to \"off\", the level 2.x applied",
        ),
        (
            MigrateText::NoteAddedRuleEntry {
                count: 1,
                at: s("the top level"),
            },
            "added 1 rule entry at the top level: 2.x applied these profile defaults over the authored category level",
        ),
        (
            MigrateText::NoteAddedRuleEntries {
                count: 2,
                at: s("the top level"),
            },
            "added 2 rule entries at the top level: 2.x applied these profile defaults over the authored category level",
        ),
        (
            MigrateText::BackupUnverified { directory: s(".b") },
            ".b was not written by `slopvac migrate --write` in this checkout, or it changed since; nothing restored",
        ),
        (
            MigrateText::BackupExists { directory: s(".b") },
            ".b exists; run `slopvac migrate --restore` or delete it first",
        ),
        (
            MigrateText::RecordUnreadable { record: s("r") },
            "r in the per-user cache: the record of an earlier `slopvac migrate --write` does not parse, so its backup may not be restored yet; run `slopvac migrate --restore` with the version that wrote it, or remove the record by hand; nothing written",
        ),
        (
            MigrateText::BackupForeignFiles { directory: s(".b") },
            ".b holds files that `slopvac migrate --write` did not write; nothing restored",
        ),
        (
            MigrateText::BackupCopyMismatch { path: s("a.md") },
            "a.md: the backup copy does not match the record; nothing restored",
        ),
        (
            MigrateText::ReplacedTwice { kept: s("k") },
            "was replaced twice while it was checked; the original is kept as k",
        ),
        (
            MigrateText::NotRemovedKept { kept: s("k") },
            "changed after it was checked, so it was not removed; it is kept as k",
        ),
        (
            MigrateText::ReplacedWhileWrittenKept { kept: s("k") },
            "was replaced while it was written; the original is kept as k",
        ),
        (
            MigrateText::HeldChanged { path: s("p") },
            "p changed after it was checked, so it was not removed; delete it by hand",
        ),
        (
            MigrateText::OutKept { path: s("a.md") },
            "kept a.md: it changed after the migration, so the restore left it as it is",
        ),
        (
            MigrateText::OutRestored { count: 4 },
            "Restored 4 files from the backup.",
        ),
        (MigrateText::OutWarning { message: s("m") }, "warning: m"),
        (
            MigrateText::OutIncomplete {
                configs: s("slopvac.toml, sub/slopvac.toml"),
            },
            "Findings: the comparison is incomplete for slopvac.toml, sub/slopvac.toml, whose before side is approximate.",
        ),
        (
            MigrateText::ReasonDoesNotLoad { problems: s("p") },
            "it does not load in 3.0: p",
        ),
        (
            MigrateText::ReasonCategoryUnderSwitch {
                at: s("overrides[0].categories"),
                line: s("4"),
                switch: s("overrides[1]"),
            },
            "`overrides[0].categories` on line 4 sets a category that applies above the profile switch in `overrides[1]`, and `--preserve-behavior` cannot keep the 2.x levels there",
        ),
        (MigrateText::OutNote { message: s("m") }, "note: m"),
        (
            MigrateText::OutWrote {
                count: 2,
                directory: s(".b"),
            },
            "Wrote 2 files. Originals are in .b; undo with `slopvac migrate --restore`.",
        ),
        (
            MigrateText::GuideLink {
                heading: s("H"),
                url: s("u"),
                anchor: s("h"),
            },
            "Read \"H\" at u#h.",
        ),
    ];
    let registered = text_entries();
    for (text, expected) in &texts {
        assert_eq!(&text.to_string(), expected);
        assert!(registered.iter().any(|e| e.code == text.code()));
    }
    assert_eq!(registered.len(), texts.len());
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table of every published text, which reads best in one place"
)]
fn every_phrase_keeps_its_published_text_and_is_registered() {
    use MigratePhrase as P;
    let phrases = [
        (
            P::HelpAbout,
            "Migrate a 2.x project to 3.0: config, inline directives and steering blocks",
        ),
        (
            P::HelpLongAbout,
            "Shows a diff of every change by default. With --write, rewrites the files and keeps the originals in .slopvac/migrate-backup. Runs the 3.0 engine on your files before and after migration and reports the change per rule.",
        ),
        (P::HelpPath, "Project root"),
        (P::HelpWrite, "Apply the changes instead of previewing them"),
        (
            P::HelpPreserveBehavior,
            "Reproduce the 2.x profile gates with the core:legacy-<profile> style",
        ),
        (P::HelpRestore, "Undo the last --write from the backup"),
        (
            P::HelpForce,
            "With --restore, also put back files that changed since the write",
        ),
        (P::OutNothing, "Nothing to migrate."),
        (
            P::OutNoChange,
            "Findings: the 3.0 engine finds the same count per rule on your files before and after migration.",
        ),
        (
            P::OutByRule,
            "Findings by rule, from the 3.0 engine on your files before and after migration (before -> after):",
        ),
        (
            P::OutPreview,
            "Preview only. Run again with --write to apply.",
        ),
        (
            P::ReasonVersionNotInteger,
            "`schema_version` must be an integer",
        ),
        (P::ReasonNoEquivalent, "no 3.0 equivalent"),
        (
            P::ReasonNoEquivalentRewrite,
            "no 3.0 equivalent; remove the key or rewrite it by hand",
        ),
        (P::ReasonExpectedTwo, "expected 2 in a 2.x config"),
        (
            P::ReasonGuidanceOverride,
            "2.x rejected guidance inside an override, and 3.0 does too",
        ),
        (P::ReasonExpectedLevelOrTable, "expected a level or a table"),
        (P::ReasonCategoryGone, "category no longer exists in 3.0"),
        (P::ReasonRuleGone, "rule no longer exists in 3.0"),
        (
            P::ReasonUnknownCategory,
            "unknown 2.x category; the rename table has no entry",
        ),
        (
            P::ReasonUnknownRuleId,
            "unknown 2.x rule id; the rename table has no entry",
        ),
        (
            P::ReasonUnknownDirectiveId,
            "directive rule id has no 3.0 name; the rename table has no entry",
        ),
        (
            P::ReasonRuleLevelOnly,
            "a rule takes a level only; 3.0 has no other rule field",
        ),
        (P::ReasonExpectedLevel, "expected a level"),
        (P::ReasonExpectedBoolean, "expected a boolean"),
        (
            P::ReasonValeEnabled,
            "Vale is removed in 3.0; set enabled = false or delete the table",
        ),
        (
            P::ReasonValeSetting,
            "Vale is removed in 3.0; set vale.enabled = false in the same table or delete it",
        ),
        (
            P::ReasonValeTable,
            "Vale is removed in 3.0; delete the table",
        ),
        (
            P::ReasonUnknownProfile,
            "unknown 2.x profile; expected strict, normal or relaxed",
        ),
        (
            P::ReasonRejectedCategory,
            "unknown 2.x category; 2.x rejected it",
        ),
        (P::ReasonRejectedRule, "unknown 2.x rule; 2.x rejected it"),
        (
            P::BlocklistSuffix,
            "the suffix is not .toml, .json, .yml or .yaml, so 3.0 cannot load it",
        ),
        (P::BlocklistNotToml, "the file is not valid TOML"),
        (
            P::BlocklistVersionNotThree,
            "a 2.x blocklist has no `schema_version`; this value is not 3, so migrate leaves it for you to fix",
        ),
        (
            P::BlocklistUnknownPos,
            "unrecognized part of speech; use noun, verb, adjective, adverb, preposition, conjunction, pronoun or article",
        ),
        (P::BlocklistNotFound, "file not found"),
        (P::BlocklistNotRegular, "not a regular file"),
        (P::BlocklistHardLinks, "has more than one hard link"),
        (P::BlocklistNoVersion, "has no `schema_version`"),
        (
            P::BlocklistBadVersion,
            "has an unsupported `schema_version`",
        ),
        (P::BlocklistUnknownKey, "has an unknown key"),
        (P::BlocklistBadFormat, "is not valid for its format"),
        (P::BlocklistUnreadable, "cannot be read"),
        (P::PlaceTopLevel, "the top level"),
        (P::OverrideBlockOne, "1 override block"),
        (
            P::CommentGuardBlocks,
            "Added by `slopvac migrate --preserve-behavior`: 2.x ignored these rule entries\n# for these files.",
        ),
        (
            P::SteeringMarkers,
            "malformed or ambiguous slopvac markers; repair the begin/end pair first",
        ),
        (P::WhyEmptyPath, "empty path"),
        (P::WhyPathLeaves, "path leaves the project"),
        (P::WhyPathNotUtf8, "path is not valid UTF-8"),
        (P::WhyPathNewline, "newline in path"),
        (
            P::WhyComponentSymlink,
            "a path component is a symbolic link",
        ),
        (P::WhyComponentNotDir, "a path component is not a directory"),
        (P::WhySymlink, "is a symbolic link"),
        (P::WhyNotDirectory, "is not a directory"),
        (P::WhyChangedWhileCreated, "changed while it was created"),
        (P::WhyNotRegular, "is not a regular file"),
        (P::WhyChangedWhileRead, "changed while it was read"),
        (P::WhyHardLinks, "has more than one hard link"),
        (P::WhyTooLarge, "is too large to migrate"),
        (P::WhyVanishedReplaced, "disappeared while it was replaced"),
        (
            P::WhyReplacedAfterCheck,
            "was replaced after it was checked",
        ),
        (P::WhyChangedAfterCheck, "changed after it was checked"),
        (P::WhyDirectoryVanished, "its directory disappeared"),
        (P::WhyVanishedAfterCheck, "disappeared after it was checked"),
        (P::WhyAppearedAfterCheck, "appeared after it was checked"),
        (
            P::WhyReplacedWhileWritten,
            "was replaced while it was written",
        ),
        (P::WhyNamedTwice, "the plan names this file twice"),
        (
            P::WhyChangedAfterPlan,
            "changed after the migration was planned",
        ),
        (
            P::WhyChangedAfterWrite,
            "changed after the migration wrote it; rerun with --force to restore anyway",
        ),
        (
            P::WhyChangedDuringRestore,
            "changed during the restore; delete it by hand",
        ),
        (
            P::WhyGainedFiles,
            "gained files during the restore; delete it by hand",
        ),
        (P::IoNonblock, "O_NONBLOCK does not fit"),
        (P::IoNoFreeName, "no free temporary name"),
        (P::IoNoFreeBackupName, "no free backup name"),
        (
            P::IoClockBeforeEpoch,
            "the system clock is before 1970, so the backup cannot be named",
        ),
        (P::IoHoldingReplaced, "a holding directory was replaced"),
        (
            P::IoCreatedReplaced,
            "a directory this run created was replaced",
        ),
    ];
    let registered = phrase_entries();
    for (phrase, expected) in phrases {
        assert_eq!(phrase.as_static(), expected);
        assert!(registered.iter().any(|e| e.code == phrase.code()));
    }
    assert_eq!(registered.len(), phrases.len());
}
