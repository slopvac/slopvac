//! The messages `slopvac-config` reports: error codes, diagnostic text, labels and help.
//!
//! Three catalogs hold them. [`ConfigMessage`] has one entry per error code. [`ConfigText`]
//! holds the sentences with parameters that fill a diagnostic's message, label and help.
//! [`ConfigPhrase`] holds fixed phrases, which callers can read as `&'static str` through
//! [`ConfigPhrase::as_static`]. The binary registers the three through `error_entries`,
//! `text_entries` and `phrase_entries`.

use slopvac_messages::{Catalog, Entry};

/// The error codes of configuration loading and validation.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum ConfigMessage {
    /// A file could not be read.
    #[code("config-io")]
    #[message("cannot read {path}: {reason}")]
    Io { path: String, reason: String },
    /// A config file is larger than the config size cap.
    #[code("config-too-large")]
    #[message("{path} is larger than {cap} bytes")]
    TooLarge {
        path: String,
        #[count]
        cap: u64,
    },
    /// A style file is larger than the style size cap.
    #[code("style-too-large")]
    #[message("{path} is larger than {cap} bytes")]
    StyleTooLarge {
        path: String,
        #[count]
        cap: u64,
    },
    /// A document is larger than the document size cap.
    #[code("document-too-large")]
    #[message("{path} is larger than {cap} bytes")]
    DocumentTooLarge {
        path: String,
        #[count]
        cap: u64,
    },
    /// The file is not valid TOML or YAML, or its front matter does not parse.
    #[code("config-parse")]
    #[message("{message}")]
    Parse { message: String },
    /// A key that the schema does not define.
    #[code("config-unknown-key")]
    #[message("{message}")]
    UnknownKey { message: String },
    /// A value of the wrong type.
    #[code("config-invalid-type")]
    #[message("{message}")]
    InvalidType { message: String },
    /// A value of the right type that the schema refuses.
    #[code("config-invalid-value")]
    #[message("{message}")]
    InvalidValue { message: String },
    /// A required key is absent.
    #[code("config-missing-key")]
    #[message("{message}")]
    MissingKey { message: String },
    /// A 2.x config: no `schema_version`, or version 2.
    #[code("config-legacy-version")]
    #[message("{message}")]
    LegacyVersion { message: String },
    /// A `schema_version` newer than this build supports.
    #[code("config-unsupported-version")]
    #[message("{message}")]
    UnsupportedVersion { message: String },
    /// An `extends` chain that loops back on itself.
    #[code("config-extends-cycle")]
    #[message("{message}")]
    ExtendsCycle { message: String },
    /// An `extends` graph that passes a budget: depth, files applied or bytes read. The diagnostic data names the budget, the limit and the file.
    #[code("config-budget-exceeded")]
    #[message("{message}")]
    BudgetExceeded { message: String },
    /// An `extends` entry that names no readable config.
    #[code("config-extends-missing")]
    #[message("{message}")]
    ExtendsMissing { message: String },
    /// Two `[[overrides]]` or `[[genres]]` blocks with the same `files` set.
    #[code("config-duplicate-scope")]
    #[message("{message}")]
    DuplicateScope { message: String },
    /// A front-matter field that `[frontmatter] allow` does not list.
    #[code("config-frontmatter-field-denied")]
    #[message("{message}")]
    FrontmatterFieldDenied { message: String },
}

/// Sentences with parameters: the message, label and help of a diagnostic, and report lines.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum ConfigText {
    /// A file with one problem: the file and the problem.
    #[code("config-rejected-one")]
    #[text("{file}: {diagnostic}")]
    RejectedOne { file: String, diagnostic: String },
    /// A file with several problems: the file and how many.
    #[code("config-rejected-many")]
    #[text("{file}: {problems} problems")]
    RejectedMany {
        file: String,
        #[count]
        problems: usize,
    },
    /// The TOML parser refused the file.
    #[code("config-parse-toml")]
    #[text("not valid TOML: {detail}")]
    NotValidToml { detail: String },
    /// The front-matter parser refused the block.
    #[code("config-parse-frontmatter")]
    #[text("the front matter does not parse: {detail}")]
    FrontMatterUnparsed { detail: String },
    /// A table repeats a key.
    #[code("config-parse-duplicate-key")]
    #[text("duplicate key `{key}`")]
    DuplicateKey { key: String },
    /// A value has the wrong type.
    #[code("config-type-mismatch")]
    #[text("expected {expected}, found {found}")]
    TypeMismatch { expected: String, found: String },
    /// Label under a value of the wrong type.
    #[code("config-type-mismatch-label")]
    #[text("{found} here")]
    FoundHere { found: String },
    /// The accepted values of a choice.
    #[code("config-expected-one-of")]
    #[text("one of {values}")]
    ExpectedOneOf { values: String },
    /// Help that lists the accepted values.
    #[code("config-help-use-one-of")]
    #[text("use {expected}")]
    UseOneOf { expected: String },
    /// Help that suggests the closest known name.
    #[code("config-help-did-you-mean")]
    #[text("did you mean `{suggestion}`?")]
    DidYouMean { suggestion: String },
    /// A string that fails the pattern for its kind.
    #[code("config-value-not-valid")]
    #[text("`{value}` is not a valid {name}")]
    NotValid { value: String, name: String },
    /// A string where a budget number or `none` belongs.
    #[code("config-value-not-budget")]
    #[text("`{value}` is not a budget")]
    NotBudget { value: String },
    /// A string outside the accepted set.
    #[code("config-value-not-accepted")]
    #[text("`{value}` is not an accepted value")]
    NotAccepted { value: String },
    /// A number outside its allowed range.
    #[code("config-value-out-of-range")]
    #[text("{value} is out of range")]
    OutOfRange { value: String },
    /// A number that is not finite.
    #[code("config-value-not-finite")]
    #[text("{value} is not a finite number")]
    NotFinite { value: String },
    /// Label for a number with both bounds.
    #[code("config-label-range-between")]
    #[text("must be between {min} and {max}")]
    RangeBetween { min: String, max: String },
    /// Label for a number with a lower bound.
    #[code("config-label-range-at-least")]
    #[text("must be at least {min}")]
    RangeAtLeast { min: String },
    /// Label for a number with an upper bound.
    #[code("config-label-range-at-most")]
    #[text("must be at most {max}")]
    RangeAtMost { max: String },
    /// A front-matter version newer than this build reads.
    #[code("config-version-newer")]
    #[text("version {version} is newer than this slopvac reads")]
    VersionNewer { version: String },
    /// A front-matter version below the supported one.
    #[code("config-version-unsupported")]
    #[text("version {version} is not supported; this build reads version 3")]
    VersionUnsupported { version: String },
    /// A list with fewer items than its minimum.
    #[code("config-list-too-short")]
    #[text("the list needs at least {min_items} item")]
    ListTooShort {
        #[count]
        min_items: usize,
    },
    /// A required key is absent.
    #[code("config-key-missing")]
    #[text("missing required key `{key}`")]
    MissingKeyMessage { key: String },
    /// Label for a missing required key.
    #[code("config-key-missing-label")]
    #[text("`{key}` is required here")]
    MissingKeyLabel { key: String },
    /// A key that the schema does not define.
    #[code("config-key-unknown")]
    #[text("unknown key `{key}`")]
    UnknownKeyMessage { key: String },
    /// A project-wide key set where only per-file keys are allowed.
    #[code("config-key-project-wide")]
    #[text("`{key}` applies to the whole run and cannot be set in {scope}")]
    ProjectWideKey { key: String, scope: String },
    /// Help for a project-wide key set in the wrong place.
    #[code("config-key-project-wide-help")]
    #[text("set `{key}` in the project config")]
    ProjectWideHelp { key: String },
    /// A `schema_version` below 3.
    #[code("config-schema-version-legacy")]
    #[text("`schema_version = {version}` marks a 2.x config")]
    SchemaVersionLegacy { version: String },
    /// A `schema_version` above the supported one.
    #[code("config-schema-version-newer")]
    #[text("`schema_version = {version}` is newer than this slopvac reads")]
    SchemaVersionNewer { version: String },
    /// Label that names the version this build reads.
    #[code("config-schema-version-newer-label")]
    #[text("this build reads version {supported}")]
    SchemaVersionNewerLabel { supported: String },
    /// A front-matter or directive field that the project does not allow.
    #[code("config-frontmatter-denied")]
    #[text("the project does not let {what} set `{key}`")]
    FieldDenied { what: String, key: String },
    /// Help for a denied front-matter field.
    #[code("config-frontmatter-denied-help")]
    #[text("to allow `{key}`, add it to `[frontmatter] allow` in slopvac.toml")]
    FieldDeniedHelp { key: String },
    /// Two blocks with the same `files` set.
    #[code("config-scope-repeats")]
    #[text("`{table}[{index}]` repeats the `files` of `{table}[{first}]`")]
    ScopeRepeats {
        table: String,
        index: String,
        first: String,
    },
    /// A `files` glob that does not compile.
    #[code("config-pattern-files-bad")]
    #[text("a `files` pattern does not compile: {detail}")]
    FilesPatternBad { detail: String },
    /// A glob that does not compile.
    #[code("config-pattern-bad")]
    #[text("the pattern does not compile: {detail}")]
    PatternBad { detail: String },
    /// The `extends` chain is too deep.
    #[code("config-budget-depth")]
    #[text("{path} nests `extends` deeper than {limit} levels")]
    BudgetDepth {
        path: String,
        #[count]
        limit: u64,
    },
    /// Too many files applied through `extends`.
    #[code("config-budget-files")]
    #[text("{path} applies more than {limit} files through `extends`")]
    BudgetFiles {
        path: String,
        #[count]
        limit: u64,
    },
    /// Too many bytes read through `extends`.
    #[code("config-budget-bytes")]
    #[text("{path} reads more than {limit} bytes through `extends`")]
    BudgetBytes {
        path: String,
        #[count]
        limit: u64,
    },
    /// Too many bytes of config applied through `extends`.
    #[code("config-budget-applied-bytes")]
    #[text("{path} applies more than {limit} bytes of config through `extends`")]
    BudgetAppliedBytes {
        path: String,
        #[count]
        limit: u64,
    },
    /// An extended config over the size cap.
    #[code("config-extended-too-large")]
    #[text("the extended config `{name}` is larger than {cap} bytes")]
    ExtendedTooLarge {
        name: String,
        #[count]
        cap: u64,
    },
    /// An extended config that cannot be read.
    #[code("config-extended-unreadable")]
    #[text("cannot read the extended config `{path}`: {detail}")]
    ExtendedUnreadable { path: String, detail: String },
    /// A plugin reference that no active plugin provides.
    #[code("config-extends-no-pack")]
    #[text("no active plugin provides the config `{name}`")]
    NoPackConfig { name: String },
    /// A value repeated under a key that must differ between list items.
    #[code("config-key-repeated")]
    #[text("`{value}` is listed more than once; each `{key}` appears once")]
    KeyRepeated { value: String, key: String },
    /// Help for a repeated key value.
    #[code("config-help-remove-repeated-key")]
    #[text("remove the repeated `{key}`, or merge the entries")]
    HelpRemoveRepeatedKey { key: String },
    /// An `extends` chain that loops.
    #[code("config-extends-loop")]
    #[text("`extends` loops: {chain}")]
    ExtendsLoop { chain: String },
    /// A directory that `validate` cannot read.
    #[code("config-validate-unreadable-inside")]
    #[text("cannot read inside {path}: {reason}")]
    UnreadableInside { path: String, reason: String },
    /// A `terms` entry that names no file.
    #[code("config-validate-term-file-missing")]
    #[text("`terms.{key}` names `{name}`, which is not a file\n --> {file}:{line}:{column}")]
    TermFileMissing {
        key: String,
        name: String,
        file: String,
        #[count]
        line: usize,
        #[count]
        column: usize,
    },
    /// The closing line of `validate`.
    #[code("config-validate-summary")]
    #[text("checked {checked} file(s): {problems} problem(s)")]
    ValidateSummary {
        #[count]
        checked: usize,
        #[count]
        problems: usize,
    },
    /// The config root in the explain output.
    #[code("config-explain-root")]
    #[text("config root: {root}")]
    ExplainRoot { root: String },
    /// A config file in the explain output.
    #[code("config-explain-file")]
    #[text("config file: {file} ({layer})")]
    ExplainFile { file: String, layer: String },
    /// A setting, its value and where it came from.
    #[code("config-explain-setting")]
    #[text("{path} = {value}  [{origin}]")]
    ExplainSetting {
        path: String,
        value: String,
        origin: String,
    },
    /// A value that a later layer replaced.
    #[code("config-explain-replaced")]
    #[text("replaced {value}  [{origin}]")]
    ExplainReplaced { value: String, origin: String },
}

/// Fixed phrases: labels, help, kind names, reasons and command help. Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum ConfigPhrase {
    /// The kind name of a null value.
    #[code("config-kind-null")]
    #[text("null")]
    KindNull,
    /// The kind name of a boolean.
    #[code("config-kind-boolean")]
    #[text("a boolean")]
    KindBoolean,
    /// The kind name of an integer.
    #[code("config-kind-integer")]
    #[text("an integer")]
    KindInteger,
    /// The kind name of a float.
    #[code("config-kind-float")]
    #[text("a float")]
    KindFloat,
    /// The kind name of a string.
    #[code("config-kind-string")]
    #[text("a string")]
    KindString,
    /// The kind name of a date-time.
    #[code("config-kind-datetime")]
    #[text("a date-time")]
    KindDatetime,
    /// The kind name of an array.
    #[code("config-kind-array")]
    #[text("an array")]
    KindArray,
    /// The kind name of a table.
    #[code("config-kind-table")]
    #[text("a table")]
    KindTable,
    /// The accepted kind: an integer or a float.
    #[code("config-kind-number")]
    #[text("a number")]
    KindNumber,
    /// The accepted kind of a front-matter root.
    #[code("config-kind-mapping")]
    #[text("a mapping")]
    KindMapping,
    /// The accepted value of a schema version.
    #[code("config-kind-integer-three")]
    #[text("the integer 3")]
    KindIntegerThree,
    /// Why a read failed: the file does not exist.
    #[code("config-reason-not-found")]
    #[text("no such file")]
    ReasonNotFound,
    /// Why a read failed: access was refused.
    #[code("config-reason-permission-denied")]
    #[text("permission denied")]
    ReasonPermissionDenied,
    /// Why a read failed: the text is not UTF-8.
    #[code("config-reason-not-utf8")]
    #[text("not valid UTF-8")]
    ReasonNotUtf8,
    /// Why a read failed: any other cause.
    #[code("config-reason-unreadable")]
    #[text("the file cannot be read")]
    ReasonUnreadable,
    /// Why a directory walk failed.
    #[code("config-reason-directory")]
    #[text("the directory cannot be read")]
    ReasonDirectory,
    /// Names a genre block in a message.
    #[code("config-scope-genre-block")]
    #[text("a genre block")]
    ScopeGenreBlock,
    /// Names an override block in a message.
    #[code("config-scope-override")]
    #[text("an override")]
    ScopeOverride,
    /// Names front matter in a message.
    #[code("config-scope-front-matter")]
    #[text("front matter")]
    ScopeFrontMatter,
    /// Names an inline directive in a message.
    #[code("config-scope-directive")]
    #[text("an inline directive")]
    ScopeDirective,
    /// Names a plugin id in a message.
    #[code("config-noun-pack-id")]
    #[text("plugin id (lowercase letters, digits and hyphens)")]
    NounPackId,
    /// Names a plugin id that may not be `core`, in a message.
    #[code("config-noun-plugin-id")]
    #[text(
        "plugin id (lowercase letters, digits and hyphens; `core` is reserved for the built-in plugin)"
    )]
    NounPluginId,
    /// Names a genre or style reference in a message.
    #[code("config-noun-reference")]
    #[text("name, bare or as `<plugin>:<name>`")]
    NounReference,
    /// Names a rule or category key in a message.
    #[code("config-noun-rule-key")]
    #[text("rule or category name")]
    NounRuleKey,
    /// Names a relative path in a message.
    #[code("config-noun-relative-path")]
    #[text("path relative to the style file")]
    NounRelativePath,
    /// A string that must not be empty.
    #[code("config-value-empty-string")]
    #[text("the string is empty")]
    EmptyString,
    /// A list that repeats an entry.
    #[code("config-list-repeats")]
    #[text("the list repeats an entry")]
    ListRepeats,
    /// A config with no `schema_version`.
    #[code("config-no-schema-version")]
    #[text("this file has no `schema_version`, so it is a 2.x config")]
    NoSchemaVersion,
    /// A 2.x config file name.
    #[code("config-legacy-dotfile")]
    #[text("`.slopvac.toml` is the 2.x config file name")]
    LegacyDotfile,
    /// A 2.x config table in `pyproject.toml`.
    #[code("config-legacy-pyproject")]
    #[text("a `[tool.slopvac]` table is a 2.x config")]
    LegacyPyproject,
    /// An inline directive with no setting.
    #[code("config-directive-empty")]
    #[text("the directive sets nothing")]
    DirectiveEmpty,
    /// Label at the place a parser stopped.
    #[code("config-label-parser-stopped")]
    #[text("the parser stopped here")]
    LabelParserStopped,
    /// Label for a name that fails its pattern.
    #[code("config-label-not-valid-here")]
    #[text("not valid here")]
    LabelNotValidHere,
    /// Label for a value that is not accepted.
    #[code("config-label-not-accepted")]
    #[text("not accepted")]
    LabelNotAccepted,
    /// Label for an empty string.
    #[code("config-label-empty-string")]
    #[text("empty string")]
    LabelEmptyString,
    /// Label for a number that is not finite.
    #[code("config-label-must-be-finite")]
    #[text("must be finite")]
    LabelMustBeFinite,
    /// Label for a number out of range with no bound to name.
    #[code("config-label-out-of-range")]
    #[text("out of range")]
    LabelOutOfRange,
    /// Label for a list with too few items.
    #[code("config-label-empty-list")]
    #[text("empty list")]
    LabelEmptyList,
    /// Label for a repeated list entry.
    #[code("config-label-repeated")]
    #[text("repeated")]
    LabelRepeated,
    /// Label for an unknown key.
    #[code("config-label-not-known-key")]
    #[text("not a known key")]
    LabelNotKnownKey,
    /// Label for a glob that does not compile.
    #[code("config-label-bad-pattern")]
    #[text("bad pattern")]
    LabelBadPattern,
    /// Label for an entry in a list.
    #[code("config-label-listed-here")]
    #[text("listed here")]
    LabelListedHere,
    /// Label for a block that repeats an earlier `files` set.
    #[code("config-label-same-files")]
    #[text("same files as an earlier block")]
    LabelSameFiles,
    /// Label for an `extends` entry.
    #[code("config-label-extended-here")]
    #[text("extended here")]
    LabelExtendedHere,
    /// Label for a 2.x config file.
    #[code("config-label-only-toml")]
    #[text("slopvac 3 reads only `slopvac.toml`")]
    LabelOnlyToml,
    /// Label for a 2.x `schema_version`.
    #[code("config-label-only-version-3")]
    #[text("slopvac 3 reads only version 3 files")]
    LabelOnlyVersion3,
    /// Label for a version above the supported one.
    #[code("config-label-reads-version-3")]
    #[text("this build reads version 3")]
    LabelReadsVersion3,
    /// Label for a version below the supported one.
    #[code("config-label-must-be-3")]
    #[text("must be 3")]
    LabelMustBe3,
    /// Label for an empty inline directive.
    #[code("config-label-write-key-value")]
    #[text("write `key=value` after `slopvac:`")]
    LabelWriteKeyValue,
    /// Label for a denied front-matter field.
    #[code("config-label-not-in-allow")]
    #[text("not in `[frontmatter] allow`")]
    NotInAllow,
    /// Help for a 2.x config.
    #[code("config-help-migrate")]
    #[text("run `slopvac migrate` to convert it")]
    HelpMigrate,
    /// Help for a version below the supported one.
    #[code("config-help-migrate-version")]
    #[text("run `slopvac migrate` for a 2.x config")]
    HelpMigrateVersion,
    /// Help for a version above the supported one.
    #[code("config-help-upgrade")]
    #[text("upgrade slopvac")]
    HelpUpgrade,
    /// Help for a malformed budget.
    #[code("config-help-budget")]
    #[text("use a number, or `none` to clear the budget")]
    HelpBudget,
    /// Help for a malformed rule or category key.
    #[code("config-help-reference-key")]
    #[text("use `<plugin>:<id>` with a lowercase plugin id, or a bare name")]
    HelpReferenceKey,
    /// Help for a table where a rule level belongs.
    #[code("config-help-bare-level")]
    #[text("a rule takes a bare level, such as `\"core:no-dash\" = \"off\"`")]
    HelpBareLevel,
    /// Help for two blocks with the same `files` set.
    #[code("config-help-merge-blocks")]
    #[text("merge the two blocks")]
    HelpMergeBlocks,
    /// Help for an `extends` chain that is too deep.
    #[code("config-help-flatten-chain")]
    #[text("flatten the chain")]
    HelpFlattenChain,
    /// Help for too many applied files.
    #[code("config-help-remove-repeated-files")]
    #[text("remove entries that extend the same files again")]
    HelpRemoveRepeatedFiles,
    /// Help for too many bytes read.
    #[code("config-help-remove-or-split")]
    #[text("remove entries or split the files")]
    HelpRemoveOrSplit,
    /// Help for too many applied bytes.
    #[code("config-help-remove-repeated-large")]
    #[text("remove entries that extend the same large files again")]
    HelpRemoveRepeatedLarge,
    /// Help for an extended config over the size cap.
    #[code("config-help-split-file")]
    #[text("split the file or remove unused settings")]
    HelpSplitFile,
    /// Help for an unreadable extended config.
    #[code("config-help-relative-to-entry")]
    #[text("paths are relative to the file that holds the entry")]
    HelpRelativeToEntry,
    /// Help for an `extends` loop.
    #[code("config-help-break-loop")]
    #[text("remove one entry from the loop")]
    HelpBreakLoop,
    /// Command help for `validate`.
    #[code("config-validate-about")]
    #[text("Check config, front matter and inline directives without linting")]
    ValidateAbout,
    /// Help for the `paths` argument.
    #[code("config-validate-paths")]
    #[text("Documents or directories to check; none checks the config alone")]
    ValidatePaths,
    /// Help for the `--config` option.
    #[code("config-validate-config")]
    #[text("Use this config for every file and skip discovery")]
    ValidateConfig,
    /// Help for the `--schema` option.
    #[code("config-validate-schema")]
    #[text("Print a JSON Schema for editors and exit")]
    ValidateSchema,
    /// Help for the `--quiet` flag.
    #[code("config-validate-quiet")]
    #[text("Print problems only")]
    ValidateQuiet,
}

impl ConfigPhrase {
    /// The phrase as an owned string, for a diagnostic field.
    pub(crate) fn text(self) -> String {
        self.as_static().to_owned()
    }
}

/// The entries of `ConfigMessage`: one per error code.
#[must_use]
pub fn error_entries() -> &'static [Entry] {
    ConfigMessage::entries()
}

/// The entries of `ConfigText`: sentences with parameters.
#[must_use]
pub fn text_entries() -> &'static [Entry] {
    ConfigText::entries()
}

/// The entries of `ConfigPhrase`: fixed phrases.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    ConfigPhrase::entries()
}
