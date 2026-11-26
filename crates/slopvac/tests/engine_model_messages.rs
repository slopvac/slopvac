//! Published engine and model wording and its workspace registration.

use slopvac_cli::messages::catalogs;
use slopvac_messages::Kind;

#[rstest::rstest]
#[case::coded_size("engine-coded-size-error", Kind::Text, "[{code}] {message}")]
#[case::changed_file("engine-changed-file-label", Kind::Text, "changed file {path}")]
#[case::context("engine-context-error", Kind::Text, "context tagger failed: {reason}")]
#[case::bad_range(
    "engine-bad-range",
    Kind::Text,
    "diagram checker returned a bad range: {reason}"
)]
#[case::tag_lookup(
    "engine-tag-lookup-budget",
    Kind::Text,
    "context lookups passed their budget of {limit} tag looks: the document is not checked"
)]
#[case::tag_build(
    "engine-tag-build-budget",
    Kind::Text,
    "the document has more than {limit} context tags: the document is not checked"
)]
#[case::sentence_budget(
    "engine-sentence-budget",
    Kind::Text,
    "the document has more than {limit} sentences: the document is not checked"
)]
#[case::cell_budget(
    "engine-cell-budget",
    Kind::Text,
    "the document has more than {limit} table cells: the document is not checked"
)]
#[case::terms("engine-terms-error", Kind::Text, "terms file: {reason}")]
#[case::comments("engine-comments-error", Kind::Text, "comment extraction: {reason}")]
#[case::internal("engine-internal-error", Kind::Text, "internal error: {reason}")]
#[case::masked_comment(
    "engine-masked-comment-error",
    Kind::Text,
    "masked comment text: {reason}"
)]
#[case::tag_limit(
    "engine-tag-limit",
    Kind::Text,
    "the document has more than {limit} context tags"
)]
#[case::sentence_mapping(
    "engine-sentence-mapping",
    Kind::Text,
    "the sentences of a block do not map onto its projected text"
)]
#[case::detector(
    "engine-duplicate-detector",
    Kind::Text,
    "detector `{name}` is already registered"
)]
#[case::fix_rst(
    "engine-fix-rst",
    Kind::Text,
    "cannot tell the code and links of an RST file apart: {reason}"
)]
#[case::path_error("engine-path-error", Kind::Text, "{path}: {reason}")]
#[case::not_utf8("engine-not-utf8", Kind::Text, "{path}: not valid UTF-8")]
#[case::no_rule("engine-cases-no-rule", Kind::Text, "no rule {rule}")]
#[case::no_cases(
    "engine-cases-none",
    Kind::Text,
    "no rule has cases or checkable examples to run"
)]
#[case::git_spawn("engine-git-spawn", Kind::Text, "could not run git: {reason}")]
#[case::git_repository(
    "engine-git-repository",
    Kind::Text,
    "Git repository required: {reason}"
)]
#[case::git_error("engine-git-error", Kind::Text, "{reason}")]
#[case::git_revision("engine-git-revision", Kind::Text, "invalid Git revision: {reason}")]
#[case::git_malformed("engine-git-malformed", Kind::Text, "malformed Git diff: {reason}")]
#[case::git_object(
    "engine-git-object-missing",
    Kind::Text,
    "Git object missing from this clone: {reason}"
)]
#[case::git_output(
    "engine-git-output-limit",
    Kind::Text,
    "git output exceeded {limit} bytes"
)]
#[case::git_timeout(
    "engine-git-timeout",
    Kind::Text,
    "git did not finish within {timeout}"
)]
#[case::git_regular(
    "engine-git-not-regular",
    Kind::Text,
    "changed path is not a regular file: {path}"
)]
#[case::git_symlink(
    "engine-git-symlink",
    Kind::Text,
    "symlinked changed path is not supported: {path}"
)]
#[case::git_escape(
    "engine-git-escape",
    Kind::Text,
    "changed path escapes Git root: {path}"
)]
#[case::git_root(
    "engine-git-resolve-root",
    Kind::Text,
    "cannot resolve the Git root: {reason}"
)]
#[case::discover_missing(
    "engine-discover-missing",
    Kind::Text,
    "no such file or directory: {path}"
)]
#[case::glob_limit(
    "engine-glob-limit",
    Kind::Text,
    "glob {pattern} expands to more than {limit} alternatives"
)]
#[case::glob_invalid("engine-glob-invalid", Kind::Text, "invalid glob {pattern}: {reason}")]
#[case::walk_error("engine-walk-error", Kind::Text, "walking {path}: {reason}")]
#[case::exclude_invalid(
    "engine-exclude-invalid",
    Kind::Text,
    "invalid exclude pattern: {reason}"
)]
#[case::rst_missing(
    "engine-rst-missing",
    Kind::Text,
    "no Python interpreter with docutils 0.21 or later: {detail}"
)]
#[case::rst_spawn(
    "engine-rst-spawn",
    Kind::Text,
    "cannot run the RST converter {converter}: {reason}"
)]
#[case::rst_timeout(
    "engine-rst-timeout",
    Kind::Text,
    "the RST converter {converter} took longer than {timeout}"
)]
#[case::rst_output(
    "engine-rst-output-limit",
    Kind::Text,
    "the RST converter {converter} wrote more than {limit} bytes"
)]
#[case::rst_failed(
    "engine-rst-failed",
    Kind::Text,
    "the RST converter {converter} failed ({status}): {stderr}"
)]
#[case::rst_invalid(
    "engine-rst-invalid-output",
    Kind::Text,
    "the RST converter {converter} wrote output that is not valid: {reason}"
)]
#[case::locale_unknown(
    "engine-locale-unknown",
    Kind::Text,
    "locale \"{tag}\" is not known, so spelling was NOT checked. Known: {known}."
)]
#[case::locale_category(
    "engine-locale-category",
    Kind::Text,
    "the {category} category is missing, so spelling was NOT checked."
)]
#[case::locale_load(
    "engine-locale-load",
    Kind::Text,
    "the generated spelling rule failed to load: {reason}"
)]
#[case::gate_incomplete("engine-gate-incomplete", Kind::Text, "incomplete check: {reasons}")]
#[case::gate_errors("engine-gate-errors", Kind::Text, "{errors} error(s), limit {limit}")]
#[case::gate_warnings(
    "engine-gate-warnings",
    Kind::Text,
    "{warnings} warning(s), limit {limit}"
)]
#[case::gate_dashes(
    "engine-gate-dashes",
    Kind::Text,
    "{dashes} Unicode dash(es), limit {limit}"
)]
#[case::gate_density(
    "engine-gate-density",
    Kind::Text,
    "{density} severity-weighted findings per 100 words, budget {budget}"
)]
#[case::gate_score("engine-gate-score", Kind::Text, "score {score}, minimum {minimum}")]
#[case::gate_category(
    "engine-gate-category",
    Kind::Text,
    "{category} at {density} severity-weighted findings per 100 words, budget {budget}"
)]
#[case::diff_duplicate(
    "engine-diff-duplicate-path",
    Kind::Text,
    "ambiguous Git diff: duplicate path {path}"
)]
#[case::diff_header(
    "engine-diff-hunk-header",
    Kind::Text,
    "malformed hunk header: {header}"
)]
#[case::diff_range("engine-diff-hunk-range", Kind::Text, "invalid hunk range: {header}")]
#[case::diff_marker(
    "engine-diff-path-marker",
    Kind::Text,
    "malformed path marker: {marker}"
)]
#[case::diff_escape("engine-diff-path-escape", Kind::Text, "ambiguous path escape: {path}")]
#[case::locale_source(
    "engine-locale-source-label",
    Kind::Text,
    "<generated locale {tag}>.json"
)]
#[case::rst_unplaced_detail(
    "engine-rst-unplaced-detail",
    Kind::Text,
    "{count} RST node(s) have no usable source lines, so their prose was not checked (first: {first})"
)]
#[case::rst_precise_detail(
    "engine-rst-line-precise-detail",
    Kind::Text,
    "{count} RST block(s) could not be matched word by word, so findings in them point at the whole block"
)]
#[case::case_count("engine-case-count", Kind::Text, "want {want} finding(s), got {got}")]
#[case::case_range(
    "engine-case-range",
    Kind::Text,
    "want {min}..={max} finding(s), got {got}"
)]
#[case::case_contains("engine-case-contains", Kind::Text, "no finding contains {needle}")]
#[case::case_absent("engine-case-absent", Kind::Text, "a finding contains {needle}")]
#[case::case_good(
    "engine-case-good-count",
    Kind::Text,
    "the good text has {count} finding(s)"
)]
#[case::case_engine("engine-case-engine-failed", Kind::Text, "the engine failed: {reason}")]
#[case::case_failed_line("engine-case-failed-line", Kind::Text, "{rule} {origin}: {reason}")]
#[case::case_incomplete_line(
    "engine-case-incomplete-line",
    Kind::Text,
    "{rule} {origin}: incomplete: {reason}"
)]
#[case::case_summary(
    "engine-case-summary",
    Kind::Text,
    "{passed} passed, {failed} failed, {incomplete} incomplete"
)]
#[case::coverage_unmatched("engine-coverage-unmatched", Kind::Text, "{rule}: matched nothing")]
#[case::coverage_summary(
    "engine-coverage-summary",
    Kind::Text,
    "{matched} of {total} rules matched in {documents} document(s); {unmatched} matched nothing"
)]
#[case::case_argument("engine-case-argument", Kind::Text, "{argument}: {reason}")]
#[case::suppression_legacy(
    "engine-suppression-legacy",
    Kind::Text,
    "suppression names {rule}, a 2.x rule id with no plugin. Run `slopvac migrate` to qualify it as <plugin>:{rule}."
)]
#[case::suppression_unknown(
    "engine-suppression-unknown",
    Kind::Text,
    "suppression names unknown rule {rule}."
)]
#[case::suppression_missing(
    "engine-suppression-no-reason",
    Kind::Text,
    "suppression of {rule} names no reason. Add reason=<one of: {valid}>."
)]
#[case::suppression_reason(
    "engine-suppression-reason",
    Kind::Text,
    "reason \"{reason}\" is not an exception of {rule}. Valid: {valid}."
)]
#[case::vocabulary_match(
    "engine-vocabulary-match",
    Kind::Text,
    "\"{matched}\" is on this project's word blocklist as {article} {pos}."
)]
#[case::discover_unsupported(
    "engine-discover-unsupported",
    Kind::Text,
    "unsupported source language for '{path}'; supported extensions: {extensions}"
)]
#[case::discover_skipped(
    "engine-discover-skipped",
    Kind::Text,
    "unsupported source language skipped: {path} ({extensions})"
)]
#[case::rst_missing_targets(
    "engine-rst-missing-targets",
    Kind::Text,
    "RST target(s) skipped: rst2html or rst2html.py not found on PATH ({paths}); install with `pip install docutils`. slopvac 3.0 reads RST through a Python interpreter that can import docutils 0.21 or later ({detail}). Set {variable} to an absolute interpreter path to choose one."
)]
#[case::rst_unchecked_targets(
    "engine-rst-unchecked-targets",
    Kind::Text,
    "RST target(s) not checked ({paths}): {reason}"
)]
#[case::rst_override("engine-rst-override", Kind::Text, "{variable} is unusable: {reason}")]
#[case::rst_no_docutils("engine-rst-no-docutils", Kind::Text, "{path} has no docutils")]
#[case::rst_skipped("engine-rst-skipped", Kind::Text, "skipped {path}: {reason}")]
#[case::rst_docutils_absent(
    "engine-rst-docutils-absent",
    Kind::Text,
    "docutils is not installed for {path}"
)]
#[case::rst_docutils_old(
    "engine-rst-docutils-old",
    Kind::Text,
    "docutils for {path} is older than 0.21"
)]
#[case::rst_exit("engine-rst-exit", Kind::Text, "exit status {code}")]
#[case::rst_inside("engine-rst-inside-project", Kind::Text, "inside the project {root}")]
#[case::rst_override_inside(
    "engine-rst-override-inside",
    Kind::Text,
    "it is inside the project {root}"
)]
#[case::range_invalid("engine-range-invalid", Kind::Text, "{range}: {reason}")]
#[case::locale_name("engine-locale-rule-name", Kind::Text, "Use {tag} spelling")]
#[case::locale_message(
    "engine-locale-rule-message",
    Kind::Text,
    "Use the {tag} spelling \"{{replacement}}\"."
)]
#[case::locale_fix("engine-locale-rule-fix", Kind::Text, "Use the {tag} spelling.")]
#[case::diff_no_body("engine-diff-no-body", Kind::Text, "hunk has no body")]
#[case::diff_no_path("engine-diff-no-new-path", Kind::Text, "file section has no new path")]
#[case::diff_format("engine-diff-format", Kind::Text, "ambiguous Git diff format")]
#[case::diff_before_header("engine-diff-before-header", Kind::Text, "content before file header")]
#[case::diff_before_paths("engine-diff-before-paths", Kind::Text, "hunk before file paths")]
#[case::diff_overflow("engine-diff-range-overflow", Kind::Text, "hunk range overflows")]
#[case::diff_newline(
    "engine-diff-newline-marker",
    Kind::Text,
    "newline marker without hunk"
)]
#[case::diff_old_path("engine-diff-old-path", Kind::Text, "duplicate old path")]
#[case::diff_new_path("engine-diff-new-path", Kind::Text, "misplaced new path")]
#[case::diff_quote("engine-diff-quoted-path", Kind::Text, "malformed quoted path")]
#[case::diff_escape_range("engine-diff-escape-range", Kind::Text, "path escape out of range")]
#[case::diff_not_utf8("engine-diff-not-utf8", Kind::Text, "path is not valid UTF-8")]
#[case::rst_boundaries(
    "engine-rst-unknown-boundaries",
    Kind::Text,
    "the project boundaries are unknown, so no PATH entry can be trusted; the caller must give the project and target directories"
)]
#[case::locale_citation(
    "engine-locale-citation",
    Kind::Text,
    "Generated from the locale table rather than written as a rule, because the correct spelling depends on the project and a rule's substitution map has only one direction. Rule 1.14 asks for American spelling; that is `locale = \"en-US\"`, the default, rather than a rule a British English project cannot turn off."
)]
#[case::cases_about(
    "engine-cases-about",
    Kind::Text,
    "Run the test cases of the rules in a plugin"
)]
#[case::cases_dir("engine-cases-dir-help", Kind::Text, "Directory of category files")]
#[case::cases_plugin("engine-cases-plugin-help", Kind::Text, "Plugin id of the rules")]
#[case::cases_rule("engine-cases-rule-help", Kind::Text, "Run one rule only")]
#[case::cases_coverage(
    "engine-cases-coverage-help",
    Kind::Text,
    "Also report the rules that match nothing in these Markdown files"
)]
#[case::case_bad_empty("engine-case-bad-empty", Kind::Text, "the bad text has no finding")]
#[case::case_no_examples(
    "engine-case-no-examples",
    Kind::Text,
    "the rule has no cases or checkable examples"
)]
#[case::case_judgement(
    "engine-case-judgement",
    Kind::Text,
    "a judgement rule has nothing to execute"
)]
#[case::suppression_syntax(
    "engine-suppression-syntax",
    Kind::Text,
    "invalid suppression; expected <!-- slopvac-allow: rule=<plugin>:<rule-id> reason=<reason> -->."
)]
#[case::suppression_no_exceptions("engine-suppression-no-exceptions", Kind::Text, "none defined")]
#[case::rule_inactive(
    "engine-rule-inactive",
    Kind::Text,
    "the rule is switched off or does not apply under the engine settings"
)]
#[case::rule_blocklist(
    "engine-rule-needs-blocklist",
    Kind::Text,
    "a vocabulary rule needs a usable word blocklist"
)]
#[case::rule_not_run(
    "engine-rule-not-run",
    Kind::Text,
    "the engine does not run this kind, style or metric of rule"
)]
#[case::discover_excluded("engine-discover-excluded", Kind::Text, "matches an exclude pattern")]
#[case::discover_kind(
    "engine-discover-kind-skipped",
    Kind::Text,
    "not a file kind this run lints"
)]
#[case::rst_search(
    "engine-rst-search-empty",
    Kind::Text,
    "no python3 or python with docutils on a trusted PATH entry"
)]
#[case::rst_path_empty("engine-rst-path-empty", Kind::Text, "empty PATH entry")]
#[case::rst_path_relative("engine-rst-path-relative", Kind::Text, "relative PATH entry")]
#[case::rst_world("engine-rst-world-writable", Kind::Text, "world-writable directory")]
#[case::rst_import(
    "engine-rst-not-importable",
    Kind::Text,
    "docutils 0.21 or later is not importable"
)]
#[case::rst_override_relative(
    "engine-rst-override-relative",
    Kind::Text,
    "it is not an absolute path"
)]
#[case::rst_override_executable(
    "engine-rst-override-not-executable",
    Kind::Text,
    "it is not an executable file"
)]
#[case::rst_pipes("engine-rst-no-pipes", Kind::Text, "no pipes")]
#[case::rst_signal("engine-rst-signal", Kind::Text, "terminated by a signal")]
#[case::extract_failed("engine-extraction-failed", Kind::Text, "diagram extraction failed")]
#[case::range_reversed("engine-range-reversed", Kind::Text, "the range is reversed")]
#[case::range_end("engine-range-past-end", Kind::Text, "the range ends past the file")]
#[case::range_outside("engine-range-outside", Kind::Text, "the range is outside the prose")]
#[case::range_character("engine-range-character", Kind::Text, "the range splits a character")]
#[case::git_unreadable("engine-git-unreadable", Kind::Text, "git output could not be read")]
#[case::rule_style(
    "rule-style-not-implemented",
    Kind::Notice,
    "{count} active {kind} rule(s) did not run: the engine runs only the sentence style"
)]
#[case::rule_kind(
    "rule-kind-not-implemented",
    Kind::Notice,
    "{count} active {kind} rule(s) did not run: the engine does not run {kind} rules yet"
)]
#[case::comment_rules(
    "comment-rules-not-run",
    Kind::Notice,
    "{count} active rule(s) do not run on code comments, which only the tokens, pattern, substitution and capitalization rules read: {rules}"
)]
#[case::comment_rule_named(
    "comment-rule-named-not-run",
    Kind::Notice,
    "{count} rule(s) you enabled by name do not run on code comments, which only the tokens, pattern, substitution and capitalization rules read: {rules}"
)]
#[case::cases_rule_unchecked(
    "cases-rule-unchecked",
    Kind::Notice,
    "{count} active rule(s) of a mechanically checkable kind were not checked by the case runner"
)]
#[case::vocabulary_no_blocklist(
    "vocabulary-no-blocklist",
    Kind::Notice,
    "{count} active vocabulary rule(s) did not run: no word blocklist is set"
)]
#[case::vocabulary_no_taggable(
    "vocabulary-no-taggable-entries",
    Kind::Notice,
    "{count} active vocabulary rule(s) did not run: the blocklist holds no noun, verb, adjective or adverb entry of three or more letters"
)]
#[case::vocabulary_shared(
    "vocabulary-sweep-shared",
    Kind::Notice,
    "{count} other vocabulary rule(s) did not run separately: the blocklist sweep runs once, under {owner}"
)]
#[case::rst_unplaced("rst-prose-unplaced", Kind::Notice, "{message}")]
#[case::rst_line_precise("rst-line-precise", Kind::Notice, "{message}")]
#[case::rst_converter_missing("rst-converter-missing", Kind::Notice, "{message}")]
#[case::rst_unchecked("rst-unchecked", Kind::Notice, "{message}")]
#[case::extract_unsupported(
    "extract-unsupported",
    Kind::Notice,
    "{reason}; the diagram was not checked"
)]
#[case::extract_malformed(
    "extract-malformed",
    Kind::Notice,
    "{reason}; the diagram was not fully checked"
)]
#[case::extract_budget(
    "extract-budget",
    Kind::Notice,
    "the {cap} limit of {limit} was reached; the diagram was not fully checked"
)]
#[case::extract_output("extract-output-invalid", Kind::Notice, "{reason}")]
#[case::fix_outside("fix-outside-root", Kind::Error, "fix-outside-root")]
#[case::fix_symlink("fix-symlink", Kind::Error, "fix-symlink")]
#[case::fix_regular("fix-not-regular", Kind::Error, "fix-not-regular")]
#[case::fix_hardlinked("fix-hardlinked", Kind::Error, "fix-hardlinked")]
#[case::fix_owner("fix-not-owner", Kind::Error, "fix-not-owner")]
#[case::fix_directory("fix-unsafe-directory", Kind::Error, "fix-unsafe-directory")]
#[case::fix_group("fix-group-mismatch", Kind::Error, "fix-group-mismatch")]
#[case::fix_acl("fix-acl-present", Kind::Error, "fix-acl-present")]
#[case::fix_platform("fix-unsupported-platform", Kind::Error, "fix-unsupported-platform")]
#[case::fix_changed("fix-changed", Kind::Error, "fix-changed")]
#[case::model_id("model-invalid-id", Kind::Text, "invalid {kind} {value}")]
#[case::model_value("model-invalid-value", Kind::Text, "invalid {kind}: {value}")]
#[case::model_duplicate(
    "model-duplicate-context",
    Kind::Text,
    "context tag {tag} is listed twice"
)]
#[case::model_version(
    "model-schema-version",
    Kind::Text,
    "schema_version must be {expected}, got {actual}"
)]
#[case::model_locator("model-origin-locator", Kind::Text, "invalid origin locator: {reason}")]
#[case::model_label("model-origin-label", Kind::Text, "invalid origin label: {reason}")]
#[case::model_segment_id("model-segment-id", Kind::Text, "invalid segment id")]
#[case::model_segment_range(
    "model-segment-range",
    Kind::Text,
    "segment range start {start} is after end {end}"
)]
#[case::model_span(
    "model-span-reversed",
    Kind::Text,
    "span start {start} is after end {end}"
)]
#[case::model_position(
    "model-line-column",
    Kind::Text,
    "line and column are one-based, got {line}:{column}"
)]
#[case::model_offset_end(
    "model-offset-past-end",
    Kind::Text,
    "offset {offset} is past the end of the file"
)]
#[case::model_offset_bom(
    "model-offset-bom",
    Kind::Text,
    "offset {offset} is inside the byte order mark"
)]
#[case::model_offset_point(
    "model-offset-code-point",
    Kind::Text,
    "offset {offset} is inside a code point"
)]
#[case::model_offset_crlf(
    "model-offset-crlf",
    Kind::Text,
    "offset {offset} is between the CR and LF of one CRLF"
)]
#[case::model_edit_position("model-edit-position", Kind::Text, "invalid edit span: {reason}")]
#[case::model_edit_overlap(
    "model-edit-overlap",
    Kind::Text,
    "edits at {first} and {second} overlap"
)]
#[case::model_fix_empty("model-fix-empty", Kind::Text, "a fix needs at least one edit")]
#[case::model_finding_range(
    "model-finding-range",
    Kind::Text,
    "span, end_line and end_column must all be present or all absent"
)]
#[case::model_finding_origin(
    "model-finding-origin-fix",
    Kind::Text,
    "a finding with an origin cannot carry a fix"
)]
#[case::model_gate_empty(
    "model-gate-empty",
    Kind::Text,
    "a failed gate needs at least one reason"
)]
#[case::model_gate_reason(
    "model-gate-empty-reason",
    Kind::Text,
    "a gate reason must not be empty"
)]
#[case::model_gate_status(
    "model-gate-inconsistent",
    Kind::Text,
    "passed is {passed} but there are {count} reasons"
)]
#[case::model_empty("model-reason-empty", Kind::Text, "empty")]
#[case::model_long("model-reason-too-long", Kind::Text, "too long")]
#[case::model_locator_character(
    "model-reason-locator-character",
    Kind::Text,
    "forbidden character, percent-escape it"
)]
#[case::model_locator_escape(
    "model-reason-locator-escape",
    Kind::Text,
    "`%` must start %XX with uppercase hex"
)]
#[case::model_label_character("model-reason-label-character", Kind::Text, "forbidden character")]
#[case::model_empty_string("model-reason-empty-string", Kind::Text, "empty string")]
#[case::model_plugin_kind("model-kind-plugin", Kind::Text, "plugin id")]
#[case::model_rule_kind("model-kind-rule", Kind::Text, "rule id")]
#[case::model_context_kind("model-kind-context", Kind::Text, "context tag")]
#[case::model_api_kind("model-kind-api-version", Kind::Text, "api version")]
#[case::model_category_kind("model-kind-category", Kind::Text, "category id")]
#[case::model_genre_kind("model-kind-genre", Kind::Text, "genre id")]
#[case::model_kebab_kind("model-kind-kebab", Kind::Text, "kebab-case name")]
#[case::model_density_kind("model-kind-density", Kind::Text, "density")]
#[case::model_score_kind("model-kind-score", Kind::Text, "score")]
#[case::model_text_kind("model-kind-text", Kind::Text, "text")]
#[case::model_version_description(
    "model-schema-version-description",
    Kind::Text,
    "Root artifact schema version."
)]
#[case::model_density_description(
    "model-density-description",
    Kind::Text,
    "Findings per 100 words."
)]
#[case::model_score_description(
    "model-score-description",
    Kind::Text,
    "Quality score from 0 (worst) to 100."
)]
#[case::model_genre_description("model-genre-description", Kind::Text, "Genre id.")]
#[case::model_span_description(
    "model-span-description",
    Kind::Text,
    "Half-open range [start, end) of byte offsets into the original file bytes, BOM included. The end is exclusive, start is at most end, and both offsets fall on code point boundaries and never between the CR and LF of one CRLF. See docs/contract/positions.md."
)]
#[case::model_position_description(
    "model-position-description",
    Kind::Text,
    "One-based line and column. The column counts Unicode code points from the line start, after any BOM, and a tab counts as one. Output formats that need another unit convert from the span. See docs/contract/positions.md."
)]
fn published_text_is_registered(#[case] code: &str, #[case] kind: Kind, #[case] template: &str) {
    let entry = catalogs()
        .into_iter()
        .flat_map(|(_, entries)| entries.iter())
        .find(|entry| entry.code == code)
        .unwrap_or_else(|| panic!("unregistered {code}"));
    assert_eq!((entry.kind, entry.template), (kind, template), "{code}");
}
