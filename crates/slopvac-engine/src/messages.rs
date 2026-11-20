//! Catalogs for engine notices and text.

use slopvac_messages::{Catalog, Entry};

macro_rules! reason_helpers {
    ($($name:ident => $variant:ident),+ $(,)?) => {
        $(pub(crate) fn $name(reason: impl ToString) -> EngineText {
            EngineText::$variant { reason: reason.to_string() }
        })+
    };
}

reason_helpers! {
    context => Context, bad_range => BadRange, terms => Terms, comments => Comments,
    internal => Internal, fix_rst => FixRst, git_spawn => GitSpawn,
    git_repository => GitRepository, git_error => GitError, git_revision => GitRevision,
    git_malformed => GitMalformed, git_object_missing => GitObjectMissing,
    exclude_invalid => ExcludeInvalid, locale_load => LocaleLoad,
}

pub(crate) fn detector(name: &str) -> EngineText {
    EngineText::DuplicateDetector { name: name.into() }
}
pub(crate) fn no_rule(rule: impl ToString) -> EngineText {
    EngineText::NoRule {
        rule: rule.to_string(),
    }
}
pub(crate) fn path_error(path: &str, reason: impl ToString) -> EngineText {
    EngineText::PathError {
        path: path.into(),
        reason: reason.to_string(),
    }
}
pub(crate) fn not_utf8(path: &str) -> EngineText {
    EngineText::NotUtf8 { path: path.into() }
}
pub(crate) fn git_timeout(timeout: &std::time::Duration) -> EngineText {
    EngineText::GitTimeout {
        timeout: format!("{timeout:?}"),
    }
}
pub(crate) fn git_not_regular(path: &std::path::Path) -> EngineText {
    EngineText::GitNotRegular {
        path: path.display().to_string(),
    }
}
pub(crate) fn git_symlink(path: &std::path::Path) -> EngineText {
    EngineText::GitSymlink {
        path: path.display().to_string(),
    }
}
pub(crate) fn git_escape(path: &std::path::Path) -> EngineText {
    EngineText::GitEscape {
        path: path.display().to_string(),
    }
}
pub(crate) fn changed_file_too_large(path: &std::path::Path, cap: u64) -> String {
    document_too_large(
        EngineText::ChangedFileLabel {
            path: path.display().to_string(),
        }
        .to_string(),
        cap,
    )
}
pub(crate) fn discover_missing(path: &str) -> EngineText {
    EngineText::DiscoverMissing { path: path.into() }
}
pub(crate) fn glob_invalid(pattern: &str, reason: impl ToString) -> EngineText {
    EngineText::GlobInvalid {
        pattern: pattern.into(),
        reason: reason.to_string(),
    }
}
pub(crate) fn walk_error(path: &std::path::Path, reason: impl ToString) -> EngineText {
    EngineText::WalkError {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

pub(crate) fn unsupported_source(path: &std::path::Path, reason: &str) -> EngineText {
    EngineText::PathError {
        path: path.display().to_string(),
        reason: reason.into(),
    }
}

pub(crate) fn glob_limit(pattern: &str, limit: usize) -> EngineText {
    EngineText::GlobLimit {
        pattern: pattern.into(),
        limit,
    }
}
pub(crate) fn rst_missing(detail: &str) -> EngineText {
    EngineText::RstMissing {
        detail: detail.into(),
    }
}
pub(crate) fn rst_spawn(converter: &std::path::Path, reason: impl ToString) -> EngineText {
    EngineText::RstSpawn {
        converter: converter.display().to_string(),
        reason: reason.to_string(),
    }
}
pub(crate) fn rst_timeout(
    converter: &std::path::Path,
    timeout: &std::time::Duration,
) -> EngineText {
    EngineText::RstTimeout {
        converter: converter.display().to_string(),
        timeout: format!("{timeout:?}"),
    }
}
pub(crate) fn rst_output_limit(converter: &std::path::Path, limit: usize) -> EngineText {
    EngineText::RstOutputLimit {
        converter: converter.display().to_string(),
        limit,
    }
}
pub(crate) fn rst_failed(converter: &std::path::Path, status: &str, stderr: &str) -> EngineText {
    EngineText::RstFailed {
        converter: converter.display().to_string(),
        status: status.into(),
        stderr: stderr.into(),
    }
}
pub(crate) fn rst_invalid_output(converter: &std::path::Path, reason: &str) -> EngineText {
    EngineText::RstInvalidOutput {
        converter: converter.display().to_string(),
        reason: reason.into(),
    }
}
pub(crate) fn locale_unknown(tag: &str) -> EngineText {
    EngineText::LocaleUnknown {
        tag: tag.into(),
        known: crate::locale::Locale::known().join(", "),
    }
}
pub(crate) fn locale_category(category: &str) -> EngineText {
    EngineText::LocaleCategory {
        category: category.into(),
    }
}
pub(crate) fn override_inside(root: &std::path::Path) -> EngineDetails {
    EngineDetails::RstOverrideInside {
        root: root.display().to_string(),
    }
}

#[derive(Debug, Catalog)]
pub(crate) enum EngineText {
    /// A reader reports a coded size-cap failure.
    #[code("engine-coded-size-error")]
    #[text("[{code}] {message}")]
    CodedSizeError { code: String, message: String },
    /// A changed-file label for a size-cap failure.
    #[code("engine-changed-file-label")]
    #[text("changed file {path}")]
    ChangedFileLabel { path: String },
    /// A context tagger failed.
    #[code("engine-context-error")]
    #[text("context tagger failed: {reason}")]
    Context { reason: String },
    /// A diagram checker returned an invalid range.
    #[code("engine-bad-range")]
    #[text("diagram checker returned a bad range: {reason}")]
    BadRange { reason: String },
    /// Context lookups passed their work budget.
    #[code("engine-tag-lookup-budget")]
    #[text("context lookups passed their budget of {limit} tag looks: the document is not checked")]
    TagLookupBudget {
        #[count]
        limit: usize,
    },
    /// Building context tags passed its work budget.
    #[code("engine-tag-build-budget")]
    #[text("the document has more than {limit} context tags: the document is not checked")]
    TagBuildBudget {
        #[count]
        limit: usize,
    },
    /// A terms file failed.
    #[code("engine-terms-error")]
    #[text("terms file: {reason}")]
    Terms { reason: String },
    /// Comment extraction failed.
    #[code("engine-comments-error")]
    #[text("comment extraction: {reason}")]
    Comments { reason: String },
    /// A constructed value broke a model invariant.
    #[code("engine-internal-error")]
    #[text("internal error: {reason}")]
    Internal { reason: String },
    /// Masked comment text failed validation.
    #[code("engine-masked-comment-error")]
    #[text("masked comment text: {reason}")]
    MaskedComment { reason: String },
    /// Context tags exceeded their limit.
    #[code("engine-tag-limit")]
    #[text("the document has more than {limit} context tags")]
    TagLimit {
        #[count]
        limit: usize,
    },
    /// Sentences do not map onto the projected text.
    #[code("engine-sentence-mapping")]
    #[text("the sentences of a block do not map onto its projected text")]
    SentenceMapping,
    /// A detector is registered twice.
    #[code("engine-duplicate-detector")]
    #[text("detector `{name}` is already registered")]
    DuplicateDetector { name: String },
    /// RST could not be read before fixing.
    #[code("engine-fix-rst")]
    #[text("cannot tell the code and links of an RST file apart: {reason}")]
    FixRst { reason: String },
    /// A file operation failed.
    #[code("engine-path-error")]
    #[text("{path}: {reason}")]
    PathError { path: String, reason: String },
    /// A file is not UTF-8.
    #[code("engine-not-utf8")]
    #[text("{path}: not valid UTF-8")]
    NotUtf8 { path: String },
    /// The requested rule does not exist.
    #[code("engine-cases-no-rule")]
    #[text("no rule {rule}")]
    NoRule { rule: String },
    /// No rules have executable cases.
    #[code("engine-cases-none")]
    #[text("no rule has cases or checkable examples to run")]
    NoCases,
    /// Git could not start.
    #[code("engine-git-spawn")]
    #[text("could not run git: {reason}")]
    GitSpawn { reason: String },
    /// Changed-line scope needs a repository.
    #[code("engine-git-repository")]
    #[text("Git repository required: {reason}")]
    GitRepository { reason: String },
    /// A Git command failed.
    #[code("engine-git-error")]
    #[text("{reason}")]
    GitError { reason: String },
    /// The revision cannot be used.
    #[code("engine-git-revision")]
    #[text("invalid Git revision: {reason}")]
    GitRevision { reason: String },
    /// The diff cannot be parsed.
    #[code("engine-git-malformed")]
    #[text("malformed Git diff: {reason}")]
    GitMalformed { reason: String },
    /// The clone lacks an object.
    #[code("engine-git-object-missing")]
    #[text("Git object missing from this clone: {reason}")]
    GitObjectMissing { reason: String },
    /// Git exceeded the output cap.
    #[code("engine-git-output-limit")]
    #[text("git output exceeded {limit} bytes")]
    GitOutputLimit {
        #[count]
        limit: usize,
    },
    /// Git exceeded its deadline.
    #[code("engine-git-timeout")]
    #[text("git did not finish within {timeout}")]
    GitTimeout { timeout: String },
    /// A changed file is not regular.
    #[code("engine-git-not-regular")]
    #[text("changed path is not a regular file: {path}")]
    GitNotRegular { path: String },
    /// A changed path is a symlink.
    #[code("engine-git-symlink")]
    #[text("symlinked changed path is not supported: {path}")]
    GitSymlink { path: String },
    /// A changed path escapes the root.
    #[code("engine-git-escape")]
    #[text("changed path escapes Git root: {path}")]
    GitEscape { path: String },
    /// Git root resolution failed.
    #[code("engine-git-resolve-root")]
    #[text("cannot resolve the Git root: {reason}")]
    GitResolveRoot { reason: String },
    /// An input is missing.
    #[code("engine-discover-missing")]
    #[text("no such file or directory: {path}")]
    DiscoverMissing { path: String },
    /// A glob expands beyond its cap.
    #[code("engine-glob-limit")]
    #[text("glob {pattern} expands to more than {limit} alternatives")]
    GlobLimit {
        pattern: String,
        #[count]
        limit: usize,
    },
    /// A glob is invalid.
    #[code("engine-glob-invalid")]
    #[text("invalid glob {pattern}: {reason}")]
    GlobInvalid { pattern: String, reason: String },
    /// Directory walking failed.
    #[code("engine-walk-error")]
    #[text("walking {path}: {reason}")]
    WalkError { path: String, reason: String },
    /// An exclude pattern is invalid.
    #[code("engine-exclude-invalid")]
    #[text("invalid exclude pattern: {reason}")]
    ExcludeInvalid { reason: String },
    /// No suitable Python interpreter exists.
    #[code("engine-rst-missing")]
    #[text("no Python interpreter with docutils 0.21 or later: {detail}")]
    RstMissing { detail: String },
    /// The converter could not start.
    #[code("engine-rst-spawn")]
    #[text("cannot run the RST converter {converter}: {reason}")]
    RstSpawn { converter: String, reason: String },
    /// The converter timed out.
    #[code("engine-rst-timeout")]
    #[text("the RST converter {converter} took longer than {timeout}")]
    RstTimeout { converter: String, timeout: String },
    /// The converter exceeded its output cap.
    #[code("engine-rst-output-limit")]
    #[text("the RST converter {converter} wrote more than {limit} bytes")]
    RstOutputLimit {
        converter: String,
        #[count]
        limit: usize,
    },
    /// The converter failed.
    #[code("engine-rst-failed")]
    #[text("the RST converter {converter} failed ({status}): {stderr}")]
    RstFailed {
        converter: String,
        status: String,
        stderr: String,
    },
    /// Converter output is invalid.
    #[code("engine-rst-invalid-output")]
    #[text("the RST converter {converter} wrote output that is not valid: {reason}")]
    RstInvalidOutput { converter: String, reason: String },
    /// A locale tag is unknown.
    #[code("engine-locale-unknown")]
    #[text("locale \"{tag}\" is not known, so spelling was NOT checked. Known: {known}.")]
    LocaleUnknown { tag: String, known: String },
    /// The spelling category is absent.
    #[code("engine-locale-category")]
    #[text("the {category} category is missing, so spelling was NOT checked.")]
    LocaleCategory { category: String },
    /// A generated spelling rule failed to load.
    #[code("engine-locale-load")]
    #[text("the generated spelling rule failed to load: {reason}")]
    LocaleLoad { reason: String },
    /// An incomplete check prevents a pass.
    #[code("engine-gate-incomplete")]
    #[text("incomplete check: {reasons}")]
    GateIncomplete { reasons: String },
    /// Errors exceed the gate limit.
    #[code("engine-gate-errors")]
    #[text("{errors} error(s), limit {limit}")]
    GateErrors {
        #[count]
        errors: usize,
        #[count]
        limit: usize,
    },
    /// Warnings exceed the gate limit.
    #[code("engine-gate-warnings")]
    #[text("{warnings} warning(s), limit {limit}")]
    GateWarnings {
        #[count]
        warnings: usize,
        #[count]
        limit: usize,
    },
    /// Dashes exceed the gate limit.
    #[code("engine-gate-dashes")]
    #[text("{dashes} Unicode dash(es), limit {limit}")]
    GateDashes {
        #[count]
        dashes: usize,
        #[count]
        limit: usize,
    },
    /// Weighted density exceeds the budget.
    #[code("engine-gate-density")]
    #[text("{density} severity-weighted findings per 100 words, budget {budget}")]
    GateDensity { density: String, budget: String },
    /// The score is below the minimum.
    #[code("engine-gate-score")]
    #[text("score {score}, minimum {minimum}")]
    GateScore { score: String, minimum: String },
    /// A category exceeds its density budget.
    #[code("engine-gate-category")]
    #[text("{category} at {density} severity-weighted findings per 100 words, budget {budget}")]
    GateCategory {
        category: String,
        density: String,
        budget: String,
    },
}

#[derive(Debug, Catalog)]
pub(crate) enum EngineDetails {
    /// A diff repeats a path.
    #[code("engine-diff-duplicate-path")]
    #[text("ambiguous Git diff: duplicate path {path}")]
    DiffDuplicatePath { path: String },
    /// A hunk header is malformed.
    #[code("engine-diff-hunk-header")]
    #[text("malformed hunk header: {header}")]
    DiffHunkHeader { header: String },
    /// A hunk range is invalid.
    #[code("engine-diff-hunk-range")]
    #[text("invalid hunk range: {header}")]
    DiffHunkRange { header: String },
    /// A path marker is malformed.
    #[code("engine-diff-path-marker")]
    #[text("malformed path marker: {marker}")]
    DiffPathMarker { marker: String },
    /// A path escape is ambiguous.
    #[code("engine-diff-path-escape")]
    #[text("ambiguous path escape: {path}")]
    DiffPathEscape { path: String },
    /// The generated locale rule's source label.
    #[code("engine-locale-source-label")]
    #[text("<generated locale {tag}>.json")]
    LocaleSourceLabel { tag: String },
    /// RST nodes have no usable source lines.
    #[code("engine-rst-unplaced-detail")]
    #[text(
        "{count} RST node(s) have no usable source lines, so their prose was not checked (first: {first})"
    )]
    RstUnplaced {
        #[count]
        count: usize,
        first: String,
    },
    /// RST blocks could only be placed by line.
    #[code("engine-rst-line-precise-detail")]
    #[text(
        "{count} RST block(s) could not be matched word by word, so findings in them point at the whole block"
    )]
    RstLinePrecise {
        #[count]
        count: usize,
    },
    /// A rule case found the wrong count.
    #[code("engine-case-count")]
    #[text("want {want} finding(s), got {got}")]
    CaseCount {
        #[count]
        want: u32,
        #[count]
        got: usize,
    },
    /// A rule case found a count outside the range.
    #[code("engine-case-range")]
    #[text("want {min}..={max} finding(s), got {got}")]
    CaseRange {
        min: String,
        max: String,
        #[count]
        got: usize,
    },
    /// No finding contains the expected text.
    #[code("engine-case-contains")]
    #[text("no finding contains {needle}")]
    CaseContains { needle: String },
    /// A finding contains forbidden text.
    #[code("engine-case-absent")]
    #[text("a finding contains {needle}")]
    CaseAbsent { needle: String },
    /// A good example produced findings.
    #[code("engine-case-good-count")]
    #[text("the good text has {count} finding(s)")]
    CaseGoodCount {
        #[count]
        count: usize,
    },
    /// The engine failed during a case.
    #[code("engine-case-engine-failed")]
    #[text("the engine failed: {reason}")]
    CaseEngineFailed { reason: String },
    /// One rule case failed.
    #[code("engine-case-failed-line")]
    #[text("{rule} {origin}: {reason}")]
    CaseFailedLine {
        rule: String,
        origin: String,
        reason: String,
    },
    /// One rule case was incomplete.
    #[code("engine-case-incomplete-line")]
    #[text("{rule} {origin}: incomplete: {reason}")]
    CaseIncompleteLine {
        rule: String,
        origin: String,
        reason: String,
    },
    /// The totals of a case run.
    #[code("engine-case-summary")]
    #[text("{passed} passed, {failed} failed, {incomplete} incomplete")]
    CaseSummary {
        #[count]
        passed: usize,
        #[count]
        failed: usize,
        #[count]
        incomplete: usize,
    },
    /// A rule matched no corpus text.
    #[code("engine-coverage-unmatched")]
    #[text("{rule}: matched nothing")]
    CoverageUnmatched { rule: String },
    /// Corpus coverage totals.
    #[code("engine-coverage-summary")]
    #[text(
        "{matched} of {total} rules matched in {documents} document(s); {unmatched} matched nothing"
    )]
    CoverageSummary {
        #[count]
        matched: usize,
        #[count]
        total: usize,
        #[count]
        documents: usize,
        #[count]
        unmatched: usize,
    },
    /// A command-line argument is invalid.
    #[code("engine-case-argument")]
    #[text("{argument}: {reason}")]
    CaseArgument { argument: String, reason: String },
    /// A suppression uses an old rule identifier.
    #[code("engine-suppression-legacy")]
    #[text(
        "suppression names {rule}, a 2.x rule id with no plugin. Run `slopvac migrate` to qualify it as <plugin>:{rule}."
    )]
    SuppressionLegacy { rule: String },
    /// A suppression names an unknown rule.
    #[code("engine-suppression-unknown")]
    #[text("suppression names unknown rule {rule}.")]
    SuppressionUnknown { rule: String },
    /// A suppression lacks a reason.
    #[code("engine-suppression-no-reason")]
    #[text("suppression of {rule} names no reason. Add reason=<one of: {valid}>.")]
    SuppressionNoReason { rule: String, valid: String },
    /// A suppression reason is invalid.
    #[code("engine-suppression-reason")]
    #[text("reason \"{reason}\" is not an exception of {rule}. Valid: {valid}.")]
    SuppressionReason {
        reason: String,
        rule: String,
        valid: String,
    },
    /// A vocabulary match is on the blocklist.
    #[code("engine-vocabulary-match")]
    #[text("\"{matched}\" is on this project's word blocklist as {article} {pos}.")]
    VocabularyMatch {
        matched: String,
        article: String,
        pos: String,
    },
    /// A source language is unsupported.
    #[code("engine-discover-unsupported")]
    #[text("unsupported source language for '{path}'; supported extensions: {extensions}")]
    DiscoverUnsupported { path: String, extensions: String },
    /// An unsupported language was skipped.
    #[code("engine-discover-skipped")]
    #[text("unsupported source language skipped: {path} ({extensions})")]
    DiscoverSkipped { path: String, extensions: String },
    /// RST targets need a converter.
    #[code("engine-rst-missing-targets")]
    #[text(
        "RST target(s) skipped: rst2html or rst2html.py not found on PATH ({paths}); install with `pip install docutils`. slopvac 3.0 reads RST through a Python interpreter that can import docutils 0.21 or later ({detail}). Set {variable} to an absolute interpreter path to choose one."
    )]
    RstMissingTargets {
        paths: String,
        detail: String,
        variable: String,
    },
    /// RST targets could not be checked.
    #[code("engine-rst-unchecked-targets")]
    #[text("RST target(s) not checked ({paths}): {reason}")]
    RstUncheckedTargets { paths: String, reason: String },
    /// The configured interpreter is unusable.
    #[code("engine-rst-override")]
    #[text("{variable} is unusable: {reason}")]
    RstOverride { variable: String, reason: String },
    /// An interpreter lacks docutils.
    #[code("engine-rst-no-docutils")]
    #[text("{path} has no docutils")]
    RstNoDocutils { path: String },
    /// An interpreter candidate was skipped.
    #[code("engine-rst-skipped")]
    #[text("skipped {path}: {reason}")]
    RstSkipped { path: String, reason: String },
    /// Docutils is not installed.
    #[code("engine-rst-docutils-absent")]
    #[text("docutils is not installed for {path}")]
    RstDocutilsAbsent { path: String },
    /// Docutils is too old.
    #[code("engine-rst-docutils-old")]
    #[text("docutils for {path} is older than 0.21")]
    RstDocutilsOld { path: String },
    /// The converter exited with a code.
    #[code("engine-rst-exit")]
    #[text("exit status {code}")]
    RstExit { code: String },
    /// An interpreter lives inside the project.
    #[code("engine-rst-inside-project")]
    #[text("inside the project {root}")]
    RstInsideProject { root: String },
    /// An explicit interpreter lives inside the project.
    #[code("engine-rst-override-inside")]
    #[text("it is inside the project {root}")]
    RstOverrideInside { root: String },
    /// A diagram range failed validation.
    #[code("engine-range-invalid")]
    #[text("{range}: {reason}")]
    RangeInvalid { range: String, reason: String },
    /// A locale's generated rule name.
    #[code("engine-locale-rule-name")]
    #[text("Use {tag} spelling")]
    LocaleRuleName { tag: String },
    /// A locale's generated rule message.
    #[code("engine-locale-rule-message")]
    #[text("Use the {tag} spelling \"{{replacement}}\".")]
    LocaleRuleMessage { tag: String },
    /// A locale's generated fix guidance.
    #[code("engine-locale-rule-fix")]
    #[text("Use the {tag} spelling.")]
    LocaleRuleFix { tag: String },
}

#[derive(Debug, Catalog)]
pub(crate) enum EnginePhrase {
    /// A hunk has no body.
    #[code("engine-diff-no-body")]
    #[text("hunk has no body")]
    DiffNoBody,
    /// A file section lacks its new path.
    #[code("engine-diff-no-new-path")]
    #[text("file section has no new path")]
    DiffNoNewPath,
    /// The diff format is ambiguous.
    #[code("engine-diff-format")]
    #[text("ambiguous Git diff format")]
    DiffFormat,
    /// Content precedes the file header.
    #[code("engine-diff-before-header")]
    #[text("content before file header")]
    DiffBeforeHeader,
    /// A hunk precedes its paths.
    #[code("engine-diff-before-paths")]
    #[text("hunk before file paths")]
    DiffBeforePaths,
    /// A hunk range overflows.
    #[code("engine-diff-range-overflow")]
    #[text("hunk range overflows")]
    DiffRangeOverflow,
    /// A newline marker has no hunk.
    #[code("engine-diff-newline-marker")]
    #[text("newline marker without hunk")]
    DiffNewlineMarker,
    /// A diff repeats its old path.
    #[code("engine-diff-old-path")]
    #[text("duplicate old path")]
    DiffOldPath,
    /// A new path is misplaced.
    #[code("engine-diff-new-path")]
    #[text("misplaced new path")]
    DiffNewPath,
    /// A quoted path is malformed.
    #[code("engine-diff-quoted-path")]
    #[text("malformed quoted path")]
    DiffQuotedPath,
    /// A path escape exceeds the byte range.
    #[code("engine-diff-escape-range")]
    #[text("path escape out of range")]
    DiffEscapeRange,
    /// A path is not UTF-8.
    #[code("engine-diff-not-utf8")]
    #[text("path is not valid UTF-8")]
    #[cfg_attr(
        unix,
        expect(
            dead_code,
            reason = "the existing non-Unix path reader rejects non-UTF-8 paths"
        )
    )]
    DiffNotUtf8,
    /// Interpreter discovery lacks project boundaries.
    #[code("engine-rst-unknown-boundaries")]
    #[text(
        "the project boundaries are unknown, so no PATH entry can be trusted; the caller must give the project and target directories"
    )]
    RstUnknownBoundaries,
    /// The generated spelling rule's citation note.
    #[code("engine-locale-citation")]
    #[text(
        "Generated from the locale table rather than written as a rule, because the correct spelling depends on the project and a rule's substitution map has only one direction. Rule 1.14 asks for American spelling; that is `locale = \"en-US\"`, the default, rather than a rule a British English project cannot turn off."
    )]
    LocaleCitation,
    /// Help for the cases command.
    #[code("engine-cases-about")]
    #[text("Run the test cases of the rules in a plugin")]
    CasesAbout,
    /// Help for the category directory.
    #[code("engine-cases-dir-help")]
    #[text("Directory of category files")]
    CasesDirHelp,
    /// Help for the plugin argument.
    #[code("engine-cases-plugin-help")]
    #[text("Plugin id of the rules")]
    CasesPluginHelp,
    /// Help for the rule argument.
    #[code("engine-cases-rule-help")]
    #[text("Run one rule only")]
    CasesRuleHelp,
    /// Help for the coverage argument.
    #[code("engine-cases-coverage-help")]
    #[text("Also report the rules that match nothing in these Markdown files")]
    CasesCoverageHelp,
    /// A bad example produced no finding.
    #[code("engine-case-bad-empty")]
    #[text("the bad text has no finding")]
    CaseBadEmpty,
    /// A rule has no executable examples.
    #[code("engine-case-no-examples")]
    #[text("the rule has no cases or checkable examples")]
    CaseNoExamples,
    /// A judgement rule cannot be executed.
    #[code("engine-case-judgement")]
    #[text("a judgement rule has nothing to execute")]
    CaseJudgement,
    /// A suppression has invalid syntax.
    #[code("engine-suppression-syntax")]
    #[text(
        "invalid suppression; expected <!-- slopvac-allow: rule=<plugin>:<rule-id> reason=<reason> -->."
    )]
    SuppressionSyntax,
    /// No exceptions are defined.
    #[code("engine-suppression-no-exceptions")]
    #[text("none defined")]
    NoExceptions,
    /// A rule is inactive.
    #[code("engine-rule-inactive")]
    #[text("the rule is switched off or does not apply under the engine settings")]
    RuleInactive,
    /// A vocabulary rule lacks usable words.
    #[code("engine-rule-needs-blocklist")]
    #[text("a vocabulary rule needs a usable word blocklist")]
    RuleNeedsBlocklist,
    /// The engine cannot execute a rule.
    #[code("engine-rule-not-run")]
    #[text("the engine does not run this kind, style or metric of rule")]
    RuleNotRun,
    /// A target matches an exclude pattern.
    #[code("engine-discover-excluded")]
    #[text("matches an exclude pattern")]
    DiscoverExcluded,
    /// A target is not lintable in this run.
    #[code("engine-discover-kind-skipped")]
    #[text("not a file kind this run lints")]
    DiscoverKindSkipped,
    /// Interpreter discovery found no usable candidate.
    #[code("engine-rst-search-empty")]
    #[text("no python3 or python with docutils on a trusted PATH entry")]
    RstSearchEmpty,
    /// A PATH entry is empty.
    #[code("engine-rst-path-empty")]
    #[text("empty PATH entry")]
    RstPathEmpty,
    /// A PATH entry is relative.
    #[code("engine-rst-path-relative")]
    #[text("relative PATH entry")]
    RstPathRelative,
    /// A candidate directory is writable by everyone.
    #[code("engine-rst-world-writable")]
    #[text("world-writable directory")]
    RstWorldWritable,
    /// Docutils cannot be imported.
    #[code("engine-rst-not-importable")]
    #[text("docutils 0.21 or later is not importable")]
    RstNotImportable,
    /// An override path is relative.
    #[code("engine-rst-override-relative")]
    #[text("it is not an absolute path")]
    RstOverrideRelative,
    /// An override path is not executable.
    #[code("engine-rst-override-not-executable")]
    #[text("it is not an executable file")]
    RstOverrideNotExecutable,
    /// Converter pipes were unavailable.
    #[code("engine-rst-no-pipes")]
    #[text("no pipes")]
    RstNoPipes,
    /// The converter was terminated.
    #[code("engine-rst-signal")]
    #[text("terminated by a signal")]
    RstSignal,
    /// Diagram extraction failed.
    #[code("engine-extraction-failed")]
    #[text("diagram extraction failed")]
    ExtractionFailed,
    /// A checker range is reversed.
    #[code("engine-range-reversed")]
    #[text("the range is reversed")]
    RangeReversed,
    /// A checker range exceeds the file.
    #[code("engine-range-past-end")]
    #[text("the range ends past the file")]
    RangePastEnd,
    /// A checker range is outside the prose.
    #[code("engine-range-outside")]
    #[text("the range is outside the prose")]
    RangeOutside,
    /// A checker range splits a character.
    #[code("engine-range-character")]
    #[text("the range splits a character")]
    RangeCharacter,
    /// Git output could not be read.
    #[code("engine-git-unreadable")]
    #[text("git output could not be read")]
    GitUnreadable,
}

impl EnginePhrase {
    pub(crate) fn text(self) -> String {
        self.as_static().to_owned()
    }
}

macro_rules! notices {
    ($unsupported:literal, $malformed:literal, $budget:literal, $invalid:literal) => {
#[derive(Debug, Catalog)]
pub(crate) enum EngineNotice {
    /// Active capitalization rules use an unsupported style.
    #[code("rule-style-not-implemented")]
    #[notice("{count} active {kind} rule(s) did not run: the engine runs only the sentence style")]
    RuleStyle {
        #[count]
        count: usize,
        kind: String,
    },
    /// Active rules use an unsupported kind.
    #[code("rule-kind-not-implemented")]
    #[notice("{count} active {kind} rule(s) did not run: the engine does not run {kind} rules yet")]
    RuleKind {
        #[count]
        count: usize,
        kind: String,
    },
    /// Nonlexical rules do not run on comments.
    #[code("comment-rules-not-run")]
    #[notice(
        "{count} active rule(s) do not run on code comments, which only the tokens, pattern, substitution and capitalization rules read: {rules}"
    )]
    CommentRules {
        #[count]
        count: usize,
        rules: String,
    },
    /// Vocabulary rules lack a blocklist.
    #[code("vocabulary-no-blocklist")]
    #[notice("{count} active vocabulary rule(s) did not run: no word blocklist is set")]
    VocabularyNoBlocklist {
        #[count]
        count: usize,
    },
    /// The blocklist has no usable entries.
    #[code("vocabulary-no-taggable-entries")]
    #[notice(
        "{count} active vocabulary rule(s) did not run: the blocklist holds no noun, verb, adjective or adverb entry of three or more letters"
    )]
    VocabularyNoTaggable {
        #[count]
        count: usize,
    },
    /// Vocabulary rules share one sweep.
    #[code("vocabulary-sweep-shared")]
    #[notice(
        "{count} other vocabulary rule(s) did not run separately: the blocklist sweep runs once, under {owner}"
    )]
    VocabularyShared {
        #[count]
        count: usize,
        owner: String,
    },
    /// RST nodes have no usable source lines.
    #[code("rst-prose-unplaced")]
    #[notice("{message}")]
    RstUnplaced { message: String },
    /// RST blocks could only be placed by line.
    #[code("rst-line-precise")]
    #[notice("{message}")]
    RstLinePrecise { message: String },
    /// No RST converter was available.
    #[code("rst-converter-missing")]
    #[notice("{message}")]
    RstConverterMissing { message: String },
    /// The RST converter failed.
    #[code("rst-unchecked")]
    #[notice("{message}")]
    RstUnchecked { message: String },
    /// Valid diagram content is unsupported.
    #[code($unsupported)]
    #[notice("{reason}; the diagram was not checked")]
    ExtractUnsupported { reason: String },
    /// A diagram is malformed.
    #[code($malformed)]
    #[notice("{reason}; the diagram was not fully checked")]
    ExtractMalformed { reason: String },
    /// A diagram extractor reached a limit.
    #[code($budget)]
    #[notice("the {cap} limit of {limit} was reached; the diagram was not fully checked")]
    ExtractBudget {
        cap: String,
        #[count]
        limit: u64,
    },
    /// An extractor plugin returned invalid output.
    #[code($invalid)]
    #[notice("{reason}")]
    #[expect(dead_code, reason = "reserved for extractor plugin protocol failures; built-in extractors do not emit this notice")]
    ExtractOutputInvalid { reason: String },
}

/// Notice codes for input an extractor could not read in full.
pub mod extract_code {
    /// Valid content that the extractor does not support; the run stays complete.
    pub const UNSUPPORTED: &str = $unsupported;
    /// Malformed input; the run is incomplete.
    pub const MALFORMED: &str = $malformed;
    /// A size or work limit stopped extraction; the run is incomplete.
    pub const BUDGET: &str = $budget;
    /// A plugin returned invalid output. Reserved for plugin extractors.
    pub const OUTPUT_INVALID: &str = $invalid;
}
    };
}

notices!(
    "extract-unsupported",
    "extract-malformed",
    "extract-budget",
    "extract-output-invalid"
);

/// Engine text registered by the binary.
#[must_use]
pub fn text_entries() -> &'static [Entry] {
    EngineText::entries()
}

/// Engine notices registered by the binary.
#[must_use]
pub fn notice_entries() -> &'static [Entry] {
    EngineNotice::entries()
}

/// Engine detail text registered by the binary.
#[must_use]
pub fn detail_entries() -> &'static [Entry] {
    EngineDetails::entries()
}

/// Engine fixed phrases registered by the binary.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    EnginePhrase::entries()
}

pub(crate) fn document_too_large(path: String, cap: u64) -> String {
    let message = slopvac_config::document_size_message(path, cap);
    EngineText::CodedSizeError {
        code: message.code().into(),
        message: message.to_string(),
    }
    .to_string()
}

#[derive(Debug, Catalog)]
pub(crate) enum FixRefusal {
    /// A fix target is outside the project root.
    #[code("fix-outside-root")]
    #[message("fix-outside-root")]
    OutsideRoot,
    /// A fix target is a symlink.
    #[code("fix-symlink")]
    #[message("fix-symlink")]
    Symlink,
    /// A fix target is not regular.
    #[code("fix-not-regular")]
    #[message("fix-not-regular")]
    NotRegular,
    /// A fix target has multiple hard links.
    #[code("fix-hardlinked")]
    #[message("fix-hardlinked")]
    Hardlinked,
    /// A fix target has a different owner.
    #[code("fix-not-owner")]
    #[message("fix-not-owner")]
    NotOwner,
    /// A fix target directory is writable by others.
    #[code("fix-unsafe-directory")]
    #[message("fix-unsafe-directory")]
    UnsafeDirectory,
    /// A replacement cannot retain the group.
    #[code("fix-group-mismatch")]
    #[message("fix-group-mismatch")]
    GroupMismatch,
    /// A fix target has an access control list.
    #[code("fix-acl-present")]
    #[message("fix-acl-present")]
    AclPresent,
    /// This platform lacks safe in-place writes.
    #[code("fix-unsupported-platform")]
    #[message("fix-unsupported-platform")]
    UnsupportedPlatform,
    /// A fix target changed before commit.
    #[code("fix-changed")]
    #[message("fix-changed")]
    Changed,
}

/// Fix refusal codes registered by the binary.
#[must_use]
pub fn refusal_entries() -> &'static [Entry] {
    FixRefusal::entries()
}
