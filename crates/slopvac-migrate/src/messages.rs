//! The messages `slopvac-migrate` reports: error codes, notices, sentences and fixed phrases.
//!
//! Three catalogs hold them. [`MigrateMessage`] has one entry per code that `Error::code` gives
//! out and the two removal notices. `MigrateText` holds sentences with parameters.
//! `MigratePhrase` holds fixed phrases, which callers read as `&'static str` through
//! `as_static`. The binary registers the three through `error_entries`, `text_entries` and
//! `phrase_entries`.
//!
//! Four published codes belong to `slopvac-config`'s catalog, and migrate reports them with its
//! own wording, held here as text: `config-parse`, `config-invalid-type`, `document-too-large`
//! and `config-too-large`.

use slopvac_messages::{Catalog, Entry};
use slopvac_terms::TermsError;

use crate::{DROPPED_SEPARATOR, Error};

/// The error codes of a migration and the two removal notices.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum MigrateMessage {
    /// Settings with no 3.0 equivalent; nothing is written. The records follow one per line.
    #[code("config-legacy-dropped")]
    #[message("{records}")]
    LegacyDropped { records: String },
    /// The config already carries `schema_version = 3`.
    #[code("config-already-migrated")]
    #[message("{file}: already migrated (schema_version is 3 or higher)")]
    AlreadyMigrated { file: String },
    /// Two config files in one directory make the result ambiguous.
    #[code("migrate-config-conflict")]
    #[message("{detail}")]
    ConfigConflict { detail: String },
    /// A path leaves the project, crosses a link or shares an inode, or a backup is in the way.
    #[code("migrate-unsafe-path")]
    #[message("unsafe: {detail}")]
    UnsafePath { detail: String },
    /// A file changed after the migration was planned or written.
    #[code("migrate-file-changed")]
    #[message("changed: {detail}")]
    FileChanged { detail: String },
    /// An override block whose `files` pattern is outside the grammar `--preserve-behavior` reads.
    #[code("migrate-override-overlap")]
    #[message(
        "{file}:{line}: `overrides[{block}]`: cannot decide how this block overlaps the others: {reason}. \
         --preserve-behavior accepts one pattern per block: an anchored path such as `docs/api` \
         or a tree such as `docs/**`"
    )]
    OverrideOverlap {
        file: String,
        line: String,
        block: String,
        reason: String,
    },
    /// The rename table is malformed.
    #[code("migrate-rename-table")]
    #[message("rename table: {detail}")]
    RenameTable { detail: String },
    /// The engine could not lint the project.
    #[code("migrate-lint-failed")]
    #[message("lint: {detail}")]
    LintFailed { detail: String },
    /// A config that the migration would write is refused: it does not load in 3.0, or it
    /// would not keep the 2.x levels. Nothing is written.
    #[code("migrate-output-invalid")]
    #[message("{file}: the migrated config is refused, so nothing is written: {reason}")]
    OutputInvalid { file: String, reason: String },
    /// A run read more files or bytes than its limits allow. `what` is `files` or `bytes`.
    #[code("migrate-run-limit")]
    #[message("{path}: run limit reached: more than {limit} {what} in one run")]
    RunLimit {
        path: String,
        #[count]
        limit: u64,
        what: String,
    },
    /// A file is not valid UTF-8.
    #[code("migrate-not-utf8")]
    #[message("{path}: not valid UTF-8")]
    NotUtf8 { path: String },
    /// A filesystem operation failed.
    #[code("migrate-io-failed")]
    #[message("{path}: {reason}")]
    IoFailed { path: String, reason: String },
    /// A rule that 3.0 no longer has was dropped. The run goes on.
    #[code("migrate-rule-removed")]
    #[notice(
        "migrate-rule-removed: {setting}; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
    )]
    RuleRemoved { setting: String },
    /// A category that 3.0 no longer has was dropped. The run goes on.
    #[code("migrate-category-removed")]
    #[notice(
        "migrate-category-removed: {setting}; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
    )]
    CategoryRemoved { setting: String },
}

/// Sentences with parameters: failures that carry a published code from another catalog,
/// notes, warnings, reasons and the lines of command output.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum MigrateText {
    /// A config is not valid TOML. Reported as `config-parse`.
    #[code("migrate-parse-failed")]
    #[text("{file}: {message}")]
    ParseFailed { file: String, message: String },
    /// `schema_version` is not an integer. Reported as `config-invalid-type`.
    #[code("migrate-invalid-type")]
    #[text("{file}:{line}: {message}")]
    InvalidType {
        file: String,
        line: String,
        message: String,
    },
    /// A document or config is larger than its cap. Reported as `document-too-large`, `config-too-large` or `terms-file-too-large`.
    #[code("migrate-path-too-large")]
    #[text("{path}: larger than {cap} bytes")]
    PathTooLarge {
        path: String,
        #[count]
        cap: u64,
    },
    /// A path with the reason it was refused or changed.
    #[code("migrate-path-reason")]
    #[text("{path}: {reason}")]
    PathReason { path: String, reason: String },
    /// A setting of a legacy config that 3.0 cannot take.
    #[code("migrate-dropped-at")]
    #[text("{file}:{line}: `{key}`: {reason}")]
    DroppedAt {
        file: String,
        line: String,
        key: String,
        reason: String,
    },
    /// The closing record of a capped list of blocking settings.
    #[code("migrate-dropped-suppressed")]
    #[text("{count} more blocking settings suppressed")]
    DroppedSuppressed {
        #[count]
        count: usize,
    },
    /// A TOML document nests tables and arrays deeper than migrate reads. `limit` is the most levels.
    #[code("migrate-nesting-too-deep")]
    #[text("tables nest more than {limit} levels deep")]
    NestingTooDeep {
        #[count]
        limit: usize,
    },
    /// The closing line of a capped list of notes.
    #[code("migrate-suppressed-notes")]
    #[text("{count} more notes suppressed")]
    SuppressedNotes {
        #[count]
        count: usize,
    },
    /// The closing line of a capped list of warnings.
    #[code("migrate-suppressed-warnings")]
    #[text("{count} more warnings suppressed")]
    SuppressedWarnings {
        #[count]
        count: usize,
    },
    /// The directory holds more than one 2.x config source.
    #[code("migrate-config-sources")]
    #[text(
        "{directory}: more than one 2.x config source (slopvac.toml, .slopvac.toml, [tool.slopvac]); \
         2.x read only one of them. Remove the ones that are unused, then rerun"
    )]
    ConfigSources { directory: String },
    /// A symbolic link was not followed.
    #[code("migrate-not-checked-symlink")]
    #[text("{path}: not checked: symbolic link (links are not followed)")]
    NotCheckedSymlink { path: String },
    /// A file of a kind that migrate does not read.
    #[code("migrate-not-checked-kind")]
    #[text("{path}: not checked: migrate reads Markdown and MDX only, not {kind} files")]
    NotCheckedKind { path: String, kind: String },
    /// An entry that is neither a file, a directory nor a link.
    #[code("migrate-not-checked-irregular")]
    #[text("{path}: not checked: not a regular file")]
    NotCheckedIrregular { path: String },
    /// A block has more than one `files` pattern.
    #[code("migrate-overlap-many-patterns")]
    #[text("a block with {count} patterns: {patterns}; use one pattern per block")]
    OverlapManyPatterns {
        #[count]
        count: usize,
        patterns: String,
    },
    /// A `files` pattern outside the supported grammar.
    #[code("migrate-overlap-unsupported")]
    #[text("unsupported pattern {pattern}")]
    OverlapUnsupported { pattern: String },
    /// The engine reported a warning for a document.
    #[code("migrate-lint-not-fully-checked")]
    #[text("{label}: not fully checked: {code}: {message}")]
    LintNotFullyChecked {
        label: String,
        code: String,
        message: String,
    },
    /// The engine failed on a document.
    #[code("migrate-lint-document-failed")]
    #[text("{label}: {detail}")]
    LintDocumentFailed { label: String, detail: String },
    /// The removed entries of the rename table are not an array.
    #[code("migrate-rename-removed-not-array")]
    #[text("removed.{key}: expected an array")]
    RenameRemovedNotArray { key: String },
    /// The removed entries are not strings.
    #[code("migrate-rename-removed-not-strings")]
    #[text("removed.{key}: expected strings")]
    RenameRemovedNotStrings { key: String },
    /// A table of the rename table is absent.
    #[code("migrate-rename-missing-table")]
    #[text("missing [{key}] table")]
    RenameMissingTable { key: String },
    /// A rename entry is not a string.
    #[code("migrate-rename-not-string")]
    #[text("{key}.{name}: expected a string")]
    RenameNotString { key: String, name: String },
    /// A `slopvac-allow` directive, as the key of a record.
    #[code("migrate-directive-key")]
    #[text("slopvac-allow rule={id}")]
    DirectiveKey { id: String },
    /// A `slopvac-allow` directive that migrate cannot read.
    #[code("migrate-directive-invalid")]
    #[text("{path}:{line}: invalid 2.x directive, left as is")]
    DirectiveInvalid { path: String, line: String },
    /// An MDX document with a directive to change.
    #[code("migrate-directive-mdx")]
    #[text(
        "{path}:{line}: MDX is not rewritten; rename or remove this slopvac-allow directive by hand"
    )]
    DirectiveMdx { path: String, line: String },
    /// The before side could not reproduce what a 2.x config meant, so it is approximate.
    #[code("migrate-note-before-approximate")]
    #[text(
        "{file}:{line}: the before side for this config is approximate, because its 2.x \
         settings could not be reproduced: {reason}"
    )]
    NoteBeforeApproximate {
        file: String,
        line: String,
        reason: String,
    },
    /// A config file was renamed.
    #[code("migrate-note-renamed-dotfile")]
    #[text("{label}: renamed to slopvac.toml")]
    NoteRenamedDotfile { label: String },
    /// The `[tool.slopvac]` table moved to its own file.
    #[code("migrate-note-moved-pyproject")]
    #[text("{label}: moved [tool.slopvac] to slopvac.toml")]
    NoteMovedPyproject { label: String },
    /// Where a TOML parser stopped, then what it said.
    #[code("migrate-parse-at-line")]
    #[text("line {line}: {message}")]
    ParseAtLine { line: String, message: String },
    /// A 2.x category was renamed.
    #[code("migrate-note-renamed-category")]
    #[text("renamed category {from} to {to}")]
    NoteRenamedCategory { from: String, to: String },
    /// An unknown key of an extension table was dropped.
    #[code("migrate-note-dropped-extension-key")]
    #[text("dropped extension key {key}")]
    NoteDroppedExtensionKey { key: String },
    /// `default` moved up.
    #[code("migrate-note-moved-default")]
    #[text("moved {path}.default to {path}")]
    NoteMovedDefault { path: String },
    /// `allow` moved to `terms.allow`.
    #[code("migrate-note-moved-allow")]
    #[text("moved {path}.allow to terms.allow")]
    NoteMovedAllow { path: String },
    /// `path` moved to `terms.blocklists`.
    #[code("migrate-note-moved-blocklist-path")]
    #[text("moved {path}.path to terms.blocklists")]
    NoteMovedBlocklistPath { path: String },
    /// The Vale table was deleted.
    #[code("migrate-note-deleted-vale")]
    #[text("deleted {path} (Vale is removed in 3.0)")]
    NoteDeletedVale { path: String },
    /// A blocklist outside the project root.
    #[code("migrate-blocklist-outside")]
    #[text(
        "{file}:{line}: `{key}`: the blocklist is outside the project root, so migrate does not rewrite it. \
         Move it into the project, or give it `schema_version = 3` by hand"
    )]
    BlocklistOutside {
        file: String,
        line: String,
        key: String,
    },
    /// A blocklist path through a symbolic link.
    #[code("migrate-blocklist-symlink")]
    #[text(
        "{file}:{line}: `{key}`: the blocklist path crosses a symbolic link, so migrate cannot tell which file the project loads. \
         Name the file without the link"
    )]
    BlocklistSymlink {
        file: String,
        line: String,
        key: String,
    },
    /// A blocklist that cannot be migrated, with the reason.
    #[code("migrate-blocklist-blocked")]
    #[text("blocklist `{label}`: {reason}")]
    BlocklistBlocked { label: String, reason: String },
    /// A JSON or YAML blocklist that 3.0 refuses.
    #[code("migrate-blocklist-format-refused")]
    #[text(
        "3.0 refuses this .{format} blocklist ({what}). migrate rewrites TOML blocklists only. \
         Add `schema_version: 3` and long-form `pos` values by hand, or convert it to TOML"
    )]
    BlocklistFormatRefused { format: String, what: String },
    /// What is wrong with a blocklist file.
    #[code("migrate-blocklist-file-problem")]
    #[text("the file {what}")]
    BlocklistFileProblem { what: String },
    /// The loader refused the migrated blocklist.
    #[code("migrate-blocklist-still-refused")]
    #[text("3.0 refuses the migrated blocklist: {what}")]
    BlocklistStillRefused { what: String },
    /// `schema_version = 3` was added to a blocklist.
    #[code("migrate-note-added-blocklist-version")]
    #[text("{label}: added schema_version = 3")]
    NoteAddedBlocklistVersion { label: String },
    /// A `pos` value was written in its long form.
    #[code("migrate-note-pos-rewritten")]
    #[text("{label}:{line}: pos rewritten to `{long}`")]
    NotePosRewritten {
        label: String,
        line: String,
        long: String,
    },
    /// A numbered entry of a blocklist has a problem.
    #[code("migrate-blocklist-entry-numbered")]
    #[text("entry {number} {problem}")]
    BlocklistEntryNumbered { number: String, problem: String },
    /// An entry of a blocklist has a problem.
    #[code("migrate-blocklist-entry")]
    #[text("an entry {problem}")]
    BlocklistEntry { problem: String },
    /// A rule entry was dropped because a legacy style decides its level.
    #[code("migrate-note-dropped-judgement")]
    #[text("dropped {path}: a judgement rule takes its review level from the legacy style")]
    NoteDroppedJudgement { path: String },
    /// A rule entry was dropped because 2.x ignored it.
    #[code("migrate-note-dropped-ignored")]
    #[text("dropped {path}: 2.x ignored it for every file it applies to")]
    NoteDroppedIgnored { path: String },
    /// The legacy style was added to reproduce the profile.
    #[code("migrate-note-added-style")]
    #[text("added styles = [\"{style}\"] to reproduce the 2.x {profile} profile")]
    NoteAddedStyle { style: String, profile: String },
    /// A profile switch was folded into the override block that selects the profile.
    #[code("migrate-note-merged-switch")]
    #[text("merged the switch to the 2.x {profile} profile into {at}")]
    NoteMergedSwitch { profile: String, at: String },
    /// Earlier blocks that hold every file of a switching block were restated in it.
    #[code("migrate-note-restated-blocks")]
    #[text("restated in {at} the categories and rules of {blocks}, which also match its files")]
    NoteRestatedBlocks { at: String, blocks: String },
    /// A block moved after a later block that holds its files and switches their profile.
    #[code("migrate-note-moved-block")]
    #[text(
        "moved {at} after {after}, which holds its files and switches them to another 2.x profile; it drops the settings that {blocks} also sets, which won in 2.x"
    )]
    NoteMovedBlock {
        at: String,
        after: String,
        blocks: String,
    },
    /// One guard entry was folded into the block with the same files.
    #[code("migrate-note-guarded-rule-entry")]
    #[text("turned off {count} rule entry in {at} that 2.x ignored for its files")]
    NoteGuardedRuleEntry {
        #[count]
        count: usize,
        at: String,
    },
    /// Several guard entries were folded into the block with the same files.
    #[code("migrate-note-guarded-rule-entries")]
    #[text("turned off {count} rule entries in {at} that 2.x ignored for its files")]
    NoteGuardedRuleEntries {
        #[count]
        count: usize,
        at: String,
    },
    /// Override blocks were added after the others.
    #[code("migrate-note-added-guard-blocks")]
    #[text("added {blocks} after the others that turn off rule entries 2.x ignored")]
    NoteAddedGuardBlocks { blocks: String },
    /// A count of override blocks, more than one.
    #[code("migrate-override-blocks")]
    #[text("{count} override blocks")]
    OverrideBlocks {
        #[count]
        count: usize,
    },
    /// A rule level was changed to the profile default.
    #[code("migrate-note-set-rule-level")]
    #[text("set {at} to \"{level}\": the 2.x profile default replaces an entry 2.x ignored")]
    NoteSetRuleLevel { at: String, level: String },
    /// Why a migrated config is refused: a blocklist applies to some files only.
    #[code("migrate-reason-blocklist-in-override")]
    #[text(
        "`{at}` on line {line} sets a word blocklist, and the top level sets none. `--preserve-behavior` cannot keep the 2.x vocabulary levels for a blocklist that applies to some files only. A follow-up will support these layouts."
    )]
    ReasonBlocklistInOverride { at: String, line: String },
    /// Why a migrated config is refused: a blocklist and a profile switch.
    #[code("migrate-reason-blocklist-under-switch")]
    #[text(
        "`{at}` on line {line} sets `profile` in an override, whatever its value, and the project sets a word blocklist. `--preserve-behavior` cannot keep the 2.x vocabulary levels in the files that override covers. A follow-up will support these layouts."
    )]
    ReasonBlocklistUnderSwitch { at: String, line: String },
    /// The vocabulary companion style was added.
    #[code("migrate-note-added-vocabulary-style")]
    #[text(
        "added \"{style}\" to styles: 2.x ran the vocabulary rules once a word blocklist was set, at the level of the {profile} profile"
    )]
    NoteAddedVocabularyStyle { style: String, profile: String },
    /// A rule level in a moved or switching block was changed to the level 2.x applied.
    #[code("migrate-note-set-rule-level-applied")]
    #[text("set {at} to \"{level}\", the level 2.x applied")]
    NoteSetRuleLevelApplied { at: String, level: String },
    /// One rule entry was added.
    #[code("migrate-note-added-rule-entry")]
    #[text(
        "added {count} rule entry at {at}: 2.x applied these profile defaults over the authored category level"
    )]
    NoteAddedRuleEntry {
        #[count]
        count: usize,
        at: String,
    },
    /// Several rule entries were added.
    #[code("migrate-note-added-rule-entries")]
    #[text(
        "added {count} rule entries at {at}: 2.x applied these profile defaults over the authored category level"
    )]
    NoteAddedRuleEntries {
        #[count]
        count: usize,
        at: String,
    },
    /// The backup directory was not written by this tool, or changed since.
    #[code("migrate-backup-unverified")]
    #[text(
        "{directory} was not written by `slopvac migrate --write` in this checkout, or it \
         changed since; nothing restored"
    )]
    BackupUnverified { directory: String },
    /// A backup directory is in the way of `--write`.
    #[code("migrate-backup-exists")]
    #[text("{directory} exists; run `slopvac migrate --restore` or delete it first")]
    BackupExists { directory: String },
    /// The record of an earlier write exists in the per-user cache but does not parse.
    #[code("migrate-record-unreadable")]
    #[text(
        "{record} in the per-user cache: the record of an earlier `slopvac migrate --write` does not \
         parse, so its backup may not be restored yet; run `slopvac migrate --restore` with the \
         version that wrote it, or remove the record by hand; nothing written"
    )]
    RecordUnreadable { record: String },
    /// The backup directory holds files the tool did not write.
    #[code("migrate-backup-foreign-files")]
    #[text(
        "{directory} holds files that `slopvac migrate --write` did not write; \
         nothing restored"
    )]
    BackupForeignFiles { directory: String },
    /// A backup copy differs from the record.
    #[code("migrate-backup-copy-mismatch")]
    #[text("{path}: the backup copy does not match the record; nothing restored")]
    BackupCopyMismatch { path: String },
    /// A holding directory took a name twice; the original is kept.
    #[code("migrate-replaced-twice")]
    #[text("was replaced twice while it was checked; the original is kept as {kept}")]
    ReplacedTwice { kept: String },
    /// A file changed before its original could be removed; the original is kept.
    #[code("migrate-not-removed-kept")]
    #[text("changed after it was checked, so it was not removed; it is kept as {kept}")]
    NotRemovedKept { kept: String },
    /// A file was replaced while it was written; the original is kept.
    #[code("migrate-replaced-while-written-kept")]
    #[text("was replaced while it was written; the original is kept as {kept}")]
    ReplacedWhileWrittenKept { kept: String },
    /// A held original changed before it could be removed.
    #[code("migrate-held-changed")]
    #[text("{path} changed after it was checked, so it was not removed; delete it by hand")]
    HeldChanged { path: String },
    /// A restored file was left as it is.
    #[code("migrate-out-kept")]
    #[text("kept {path}: it changed after the migration, so the restore left it as it is")]
    OutKept { path: String },
    /// A restore finished.
    #[code("migrate-out-restored")]
    #[text("Restored {count} files from the backup.")]
    OutRestored {
        #[count]
        count: usize,
    },
    /// A warning line of the preview.
    #[code("migrate-out-warning")]
    #[text("warning: {message}")]
    OutWarning { message: String },
    /// The findings comparison cannot be trusted for some configs.
    #[code("migrate-out-incomplete")]
    #[text(
        "Findings: the comparison is incomplete for {configs}, whose before side is approximate."
    )]
    OutIncomplete { configs: String },
    /// A note line of the preview.
    #[code("migrate-out-note")]
    #[text("note: {message}")]
    OutNote { message: String },
    /// A write finished.
    #[code("migrate-out-wrote")]
    #[text(
        "Wrote {count} files. Originals are in {directory}; undo with `slopvac migrate --restore`."
    )]
    OutWrote {
        #[count]
        count: usize,
        directory: String,
    },
    /// The reader to a section of the guide.
    #[code("migrate-guide-link")]
    #[text("Read \"{heading}\" at {url}#{anchor}.")]
    GuideLink {
        heading: String,
        url: String,
        anchor: String,
    },
}

/// Fixed phrases: command help, reasons, advice and short failures. Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum MigratePhrase {
    /// Command help for `migrate`.
    #[code("migrate-help-about")]
    #[text("Migrate a 2.x project to 3.0: config, inline directives and steering blocks")]
    HelpAbout,
    /// Long command help for `migrate`.
    #[code("migrate-help-long-about")]
    #[text(
        "Shows a diff of every change by default. With --write, rewrites the files \
         and keeps the originals in .slopvac/migrate-backup. Runs the 3.0 engine on \
         your files before and after migration and reports the change per rule."
    )]
    HelpLongAbout,
    /// Help for the project root argument.
    #[code("migrate-help-path")]
    #[text("Project root")]
    HelpPath,
    /// Help for `--write`.
    #[code("migrate-help-write")]
    #[text("Apply the changes instead of previewing them")]
    HelpWrite,
    /// Help for `--preserve-behavior`.
    #[code("migrate-help-preserve-behavior")]
    #[text("Reproduce the 2.x profile gates with the core:legacy-<profile> style")]
    HelpPreserveBehavior,
    /// Help for `--restore`.
    #[code("migrate-help-restore")]
    #[text("Undo the last --write from the backup")]
    HelpRestore,
    /// Help for `--force`.
    #[code("migrate-help-force")]
    #[text("With --restore, also put back files that changed since the write")]
    HelpForce,
    /// Nothing to do.
    #[code("migrate-out-nothing")]
    #[text("Nothing to migrate.")]
    OutNothing,
    /// No rule changed its findings.
    #[code("migrate-out-no-change")]
    #[text(
        "Findings: the 3.0 engine finds the same count per rule on your files before and \
         after migration."
    )]
    OutNoChange,
    /// The heading of the per-rule table.
    #[code("migrate-out-by-rule")]
    #[text(
        "Findings by rule, from the 3.0 engine on your files before and after migration \
         (before -> after):"
    )]
    OutByRule,
    /// The preview ends here.
    #[code("migrate-out-preview")]
    #[text("Preview only. Run again with --write to apply.")]
    OutPreview,
    /// `schema_version` is not an integer.
    #[code("migrate-reason-version-not-integer")]
    #[text("`schema_version` must be an integer")]
    ReasonVersionNotInteger,
    /// A setting that 3.0 has no key for.
    #[code("migrate-reason-no-equivalent")]
    #[text("no 3.0 equivalent")]
    ReasonNoEquivalent,
    /// A top-level key that 3.0 has no key for.
    #[code("migrate-reason-no-equivalent-rewrite")]
    #[text("no 3.0 equivalent; remove the key or rewrite it by hand")]
    ReasonNoEquivalentRewrite,
    /// The legacy `schema_version` is not 2.
    #[code("migrate-reason-expected-two")]
    #[text("expected 2 in a 2.x config")]
    ReasonExpectedTwo,
    /// Guidance in an override.
    #[code("migrate-reason-guidance-override")]
    #[text("2.x rejected guidance inside an override, and 3.0 does too")]
    ReasonGuidanceOverride,
    /// A category entry of the wrong shape.
    #[code("migrate-reason-expected-level-or-table")]
    #[text("expected a level or a table")]
    ReasonExpectedLevelOrTable,
    /// A category that 3.0 no longer has.
    #[code("migrate-reason-category-gone")]
    #[text("category no longer exists in 3.0")]
    ReasonCategoryGone,
    /// A rule that 3.0 no longer has.
    #[code("migrate-reason-rule-gone")]
    #[text("rule no longer exists in 3.0")]
    ReasonRuleGone,
    /// A category with no entry in the rename table.
    #[code("migrate-reason-unknown-category")]
    #[text("unknown 2.x category; the rename table has no entry")]
    ReasonUnknownCategory,
    /// A rule with no entry in the rename table.
    #[code("migrate-reason-unknown-rule-id")]
    #[text("unknown 2.x rule id; the rename table has no entry")]
    ReasonUnknownRuleId,
    /// A directive rule id with no entry in the rename table.
    #[code("migrate-reason-unknown-directive-id")]
    #[text("directive rule id has no 3.0 name; the rename table has no entry")]
    ReasonUnknownDirectiveId,
    /// A rule given as something other than a level.
    #[code("migrate-reason-rule-level-only")]
    #[text("a rule takes a level only; 3.0 has no other rule field")]
    ReasonRuleLevelOnly,
    /// A value that must be a level.
    #[code("migrate-reason-expected-level")]
    #[text("expected a level")]
    ReasonExpectedLevel,
    /// A value that must be a boolean.
    #[code("migrate-reason-expected-boolean")]
    #[text("expected a boolean")]
    ReasonExpectedBoolean,
    /// Vale is enabled.
    #[code("migrate-reason-vale-enabled")]
    #[text("Vale is removed in 3.0; set enabled = false or delete the table")]
    ReasonValeEnabled,
    /// A Vale setting beside a Vale table that is not disabled.
    #[code("migrate-reason-vale-setting")]
    #[text("Vale is removed in 3.0; set vale.enabled = false in the same table or delete it")]
    ReasonValeSetting,
    /// A Vale table left to delete.
    #[code("migrate-reason-vale-table")]
    #[text("Vale is removed in 3.0; delete the table")]
    ReasonValeTable,
    /// An unknown 2.x profile.
    #[code("migrate-reason-unknown-profile")]
    #[text("unknown 2.x profile; expected strict, normal or relaxed")]
    ReasonUnknownProfile,
    /// A category that 2.x rejected.
    #[code("migrate-reason-rejected-category")]
    #[text("unknown 2.x category; 2.x rejected it")]
    ReasonRejectedCategory,
    /// A rule that 2.x rejected.
    #[code("migrate-reason-rejected-rule")]
    #[text("unknown 2.x rule; 2.x rejected it")]
    ReasonRejectedRule,
    /// A blocklist with a suffix that 3.0 does not load.
    #[code("migrate-blocklist-suffix")]
    #[text("the suffix is not .toml, .json, .yml or .yaml, so 3.0 cannot load it")]
    BlocklistSuffix,
    /// A blocklist that is not TOML.
    #[code("migrate-blocklist-not-toml")]
    #[text("the file is not valid TOML")]
    BlocklistNotToml,
    /// A blocklist with a version other than 3.
    #[code("migrate-blocklist-version-not-three")]
    #[text(
        "a 2.x blocklist has no `schema_version`; this value is not 3, so migrate leaves it for you to fix"
    )]
    BlocklistVersionNotThree,
    /// A `pos` value that no spelling matches.
    #[code("migrate-blocklist-unknown-pos")]
    #[text(
        "unrecognized part of speech; use noun, verb, adjective, adverb, \
         preposition, conjunction, pronoun or article"
    )]
    BlocklistUnknownPos,
    /// The blocklist does not exist.
    #[code("migrate-blocklist-not-found")]
    #[text("file not found")]
    BlocklistNotFound,
    /// The blocklist is not a regular file.
    #[code("migrate-blocklist-not-regular")]
    #[text("not a regular file")]
    BlocklistNotRegular,
    /// The blocklist has more than one hard link.
    #[code("migrate-blocklist-hard-links")]
    #[text("has more than one hard link")]
    BlocklistHardLinks,
    /// A blocklist with no `schema_version`.
    #[code("migrate-blocklist-no-version")]
    #[text("has no `schema_version`")]
    BlocklistNoVersion,
    /// A blocklist with a `schema_version` the loader does not take.
    #[code("migrate-blocklist-bad-version")]
    #[text("has an unsupported `schema_version`")]
    BlocklistBadVersion,
    /// A blocklist with an unknown key.
    #[code("migrate-blocklist-unknown-key")]
    #[text("has an unknown key")]
    BlocklistUnknownKey,
    /// A blocklist the parser refuses.
    #[code("migrate-blocklist-bad-format")]
    #[text("is not valid for its format")]
    BlocklistBadFormat,
    /// A blocklist that cannot be read.
    #[code("migrate-blocklist-unreadable")]
    #[text("cannot be read")]
    BlocklistUnreadable,
    /// The top level of a config, as a place.
    #[code("migrate-place-top-level")]
    #[text("the top level")]
    PlaceTopLevel,
    /// One override block.
    #[code("migrate-override-block-one")]
    #[text("1 override block")]
    OverrideBlockOne,
    /// The comment above the guard blocks of `--preserve-behavior`.
    #[code("migrate-comment-guard-blocks")]
    #[text(
        "Added by `slopvac migrate --preserve-behavior`: 2.x ignored these rule entries\n# for these files."
    )]
    CommentGuardBlocks,
    /// The markers of the steering block are damaged.
    #[code("migrate-steering-markers")]
    #[text("malformed or ambiguous slopvac markers; repair the begin/end pair first")]
    SteeringMarkers,
    /// An empty path.
    #[code("migrate-why-empty-path")]
    #[text("empty path")]
    WhyEmptyPath,
    /// A path leaves the project.
    #[code("migrate-why-path-leaves")]
    #[text("path leaves the project")]
    WhyPathLeaves,
    /// A path is not UTF-8.
    #[code("migrate-why-path-not-utf8")]
    #[text("path is not valid UTF-8")]
    WhyPathNotUtf8,
    /// A path holds a newline.
    #[code("migrate-why-path-newline")]
    #[text("newline in path")]
    WhyPathNewline,
    /// A path component is a symbolic link.
    #[code("migrate-why-component-symlink")]
    #[text("a path component is a symbolic link")]
    WhyComponentSymlink,
    /// The entry is not a directory.
    #[code("migrate-why-not-directory")]
    #[text("is not a directory")]
    WhyNotDirectory,
    /// A directory changed while this run created it.
    #[code("migrate-why-changed-while-created")]
    #[text("changed while it was created")]
    WhyChangedWhileCreated,
    /// A path component is not a directory.
    #[code("migrate-why-component-not-dir")]
    #[text("a path component is not a directory")]
    WhyComponentNotDir,
    /// The file is a symbolic link.
    #[code("migrate-why-symlink")]
    #[text("is a symbolic link")]
    WhySymlink,
    /// The file is not a regular file.
    #[code("migrate-why-not-regular")]
    #[text("is not a regular file")]
    WhyNotRegular,
    /// The file changed while it was read.
    #[code("migrate-why-changed-while-read")]
    #[text("changed while it was read")]
    WhyChangedWhileRead,
    /// The file has more than one hard link.
    #[code("migrate-why-hard-links")]
    #[text("has more than one hard link")]
    WhyHardLinks,
    /// The file is over the cap.
    #[code("migrate-why-too-large")]
    #[text("is too large to migrate")]
    WhyTooLarge,
    /// The file vanished while it was replaced.
    #[code("migrate-why-vanished-replaced")]
    #[text("disappeared while it was replaced")]
    WhyVanishedReplaced,
    /// The file was replaced after the check.
    #[code("migrate-why-replaced-after-check")]
    #[text("was replaced after it was checked")]
    WhyReplacedAfterCheck,
    /// The file changed after the check.
    #[code("migrate-why-changed-after-check")]
    #[text("changed after it was checked")]
    WhyChangedAfterCheck,
    /// The directory of the file vanished.
    #[code("migrate-why-directory-vanished")]
    #[text("its directory disappeared")]
    WhyDirectoryVanished,
    /// The file vanished after the check.
    #[code("migrate-why-vanished-after-check")]
    #[text("disappeared after it was checked")]
    WhyVanishedAfterCheck,
    /// The file appeared after the check.
    #[code("migrate-why-appeared-after-check")]
    #[text("appeared after it was checked")]
    WhyAppearedAfterCheck,
    /// The file was replaced while it was written.
    #[code("migrate-why-replaced-while-written")]
    #[text("was replaced while it was written")]
    WhyReplacedWhileWritten,
    /// The plan names a file twice.
    #[code("migrate-why-named-twice")]
    #[text("the plan names this file twice")]
    WhyNamedTwice,
    /// The file changed after the plan.
    #[code("migrate-why-changed-after-plan")]
    #[text("changed after the migration was planned")]
    WhyChangedAfterPlan,
    /// The file changed after the migration wrote it.
    #[code("migrate-why-changed-after-write")]
    #[text("changed after the migration wrote it; rerun with --force to restore anyway")]
    WhyChangedAfterWrite,
    /// The backup directory changed during the restore.
    #[code("migrate-why-changed-during-restore")]
    #[text("changed during the restore; delete it by hand")]
    WhyChangedDuringRestore,
    /// The backup directory gained files during the restore.
    #[code("migrate-why-gained-files")]
    #[text("gained files during the restore; delete it by hand")]
    WhyGainedFiles,
    /// A flag does not fit the platform's flag type.
    #[code("migrate-io-nonblock")]
    #[text("O_NONBLOCK does not fit")]
    IoNonblock,
    /// No holding-directory name was free.
    #[code("migrate-io-no-free-name")]
    #[text("no free temporary name")]
    IoNoFreeName,
    /// No backup-directory name was free.
    #[code("migrate-io-no-free-backup-name")]
    #[text("no free backup name")]
    IoNoFreeBackupName,
    /// The system clock is before the epoch, so a backup directory cannot be named.
    #[code("migrate-io-clock-before-epoch")]
    #[text("the system clock is before 1970, so the backup cannot be named")]
    IoClockBeforeEpoch,
    /// A holding directory was swapped.
    #[code("migrate-io-holding-replaced")]
    #[text("a holding directory was replaced")]
    IoHoldingReplaced,
    /// A directory this run created was swapped.
    #[code("migrate-io-created-replaced")]
    #[text("a directory this run created was replaced")]
    IoCreatedReplaced,
}

/// The entries of `MigrateMessage`: one per code, and the two removal notices.
#[must_use]
pub fn error_entries() -> &'static [Entry] {
    MigrateMessage::entries()
}

/// The entries of `MigrateText`: sentences with parameters.
#[must_use]
pub fn text_entries() -> &'static [Entry] {
    MigrateText::entries()
}

/// The entries of `MigratePhrase`: fixed phrases.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    MigratePhrase::entries()
}

/// `path` with the reason a check refused it, as one `path: reason` message.
pub(crate) fn path_reason(path: &str, reason: &str) -> String {
    MigrateText::PathReason {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
    .to_string()
}

/// The message a failure renders from this crate's catalogs, or `None` for a failure that the
/// catalog of another crate renders.
pub(crate) fn of(error: &Error) -> Option<Box<dyn Catalog>> {
    Some(match error {
        Error::Dropped(records) => Box::new(MigrateMessage::LegacyDropped {
            records: records
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(DROPPED_SEPARATOR),
        }),
        Error::Parse { file, message } => Box::new(MigrateText::ParseFailed {
            file: file.clone(),
            message: message.clone(),
        }),
        Error::AlreadyMigrated { file } => {
            Box::new(MigrateMessage::AlreadyMigrated { file: file.clone() })
        }
        Error::Conflict(detail) => Box::new(MigrateMessage::ConfigConflict {
            detail: detail.clone(),
        }),
        Error::Unsafe(detail) => Box::new(MigrateMessage::UnsafePath {
            detail: detail.clone(),
        }),
        Error::Changed(detail) => Box::new(MigrateMessage::FileChanged {
            detail: detail.clone(),
        }),
        Error::OverrideOverlap {
            file,
            line,
            block,
            reason,
        } => Box::new(MigrateMessage::OverrideOverlap {
            file: file.clone(),
            line: line.to_string(),
            block: block.to_string(),
            reason: reason.clone(),
        }),
        Error::BlocklistLayout { file, reason, .. } => Box::new(MigrateMessage::OutputInvalid {
            file: file.clone(),
            reason: reason.clone(),
        }),
        Error::RenameTable(detail) => Box::new(MigrateMessage::RenameTable {
            detail: detail.clone(),
        }),
        Error::Lint(detail) => Box::new(MigrateMessage::LintFailed {
            detail: detail.clone(),
        }),
        Error::InvalidOutput { file, cause } => Box::new(MigrateMessage::OutputInvalid {
            file: file.clone(),
            reason: cause.to_string(),
        }),
        Error::InvalidType {
            file,
            line,
            message,
        } => Box::new(MigrateText::InvalidType {
            file: file.clone(),
            line: line.to_string(),
            message: message.clone(),
        }),
        Error::Read { path, source } => Box::new(MigrateMessage::IoFailed {
            path: path.clone(),
            reason: source.to_string(),
        }),
        Error::Io { path, source } => Box::new(MigrateMessage::IoFailed {
            path: path.clone(),
            reason: source.to_string(),
        }),
        Error::DocumentTooLarge { path, cap } | Error::ConfigTooLarge { path, cap } => {
            Box::new(MigrateText::PathTooLarge {
                path: path.clone(),
                cap: *cap,
            })
        }
        Error::TermsFileTooLarge { .. } => return None,
        Error::RunLimit { path, what, limit } => Box::new(MigrateMessage::RunLimit {
            path: path.clone(),
            limit: *limit,
            what: (*what).to_owned(),
        }),
        Error::NotUtf8 { path } => Box::new(MigrateMessage::NotUtf8 { path: path.clone() }),
    })
}

/// The text of a failure, rendered through the catalog. A blocklist over the terms file cap
/// renders through the `slopvac-terms` catalog, which owns that code.
pub(crate) fn text(error: &Error) -> String {
    if let Error::TermsFileTooLarge { path, cap } = error {
        return TermsError::TooLarge {
            code: crate::TERMS_FILE_TOO_LARGE,
            path: path.clone(),
            cap: *cap,
        }
        .to_string();
    }
    of(error).map_or_else(String::new, |message| slopvac_messages::render(&*message))
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
