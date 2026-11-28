//! The `report` command's error codes, as a message catalog.
//!
//! Every failure of `slopvac report` exits 2 with one of these codes. The parameters hold fixed
//! phrases and form field ids. They never hold what the user typed or the contents of a file.

use slopvac_messages::Catalog;

/// Why `slopvac report` stopped.
#[derive(Debug, Catalog)]
pub enum ReportError {
    /// A required form field is missing.
    #[code("report-incomplete")]
    #[message(
        "required fields are missing: {fields}. Pass each as --field ID=VALUE (tick a box with ID=true)."
    )]
    Incomplete {
        /// The ids of the missing fields.
        fields: String,
    },
    /// The `--body-file` path cannot be used or written.
    #[code("report-body-file")]
    #[message("--body-file: {reason}")]
    BodyFile {
        /// A fixed phrase that names the problem.
        reason: String,
    },
    /// The GitHub CLI (`gh`) was not found.
    #[code("report-gh-missing")]
    #[message(
        "the GitHub CLI `gh` was not found on PATH. Nothing was created. Install it from https://cli.github.com/, or run the printed command yourself."
    )]
    GhMissing,
    /// The GitHub CLI is not logged in to github.com.
    #[code("report-gh-not-logged-in")]
    #[message("`gh auth status` failed; run `gh auth login`. Nothing was created.")]
    GhNotLoggedIn,
    /// `gh issue create` failed or ran longer than 60 seconds.
    #[code("report-gh-failed")]
    #[message("`gh issue create` failed: {reason}")]
    GhFailed {
        /// The exit status, or that the command timed out.
        reason: String,
    },
    /// A title, field or document sample holds a control character, or an argument is malformed.
    #[code("report-invalid")]
    #[message("invalid report input: {detail}")]
    Invalid {
        /// A fixed phrase that names the problem and the field id, never the value.
        detail: String,
    },
    /// The `--include-text` file cannot be read.
    #[code("report-sample-unreadable")]
    #[message("--include-text: {reason}")]
    SampleUnreadable {
        /// A fixed phrase that names the problem, never the file contents.
        reason: String,
    },
    /// The reporting record cannot be read or written, so it was left as it was.
    #[code("report-state-failed")]
    #[message(
        "the issue reporting record cannot be used: {reason}. It was not changed. Remove the file `reporting.json` in the slopvac state directory to start again."
    )]
    StateFailed {
        /// A fixed phrase that names the problem, never a path.
        reason: String,
    },
    /// `report consent on` or `off` ran in CI.
    #[code("report-ci-refused")]
    #[message(
        "issue reporting consent cannot be changed in CI (`CI` or `GITHUB_ACTIONS` is set). Nothing was changed. Run the command on your own machine."
    )]
    CiRefused,
}

/// Fixed phrases of `slopvac report`: command help, the reasons a failure gives, and short labels.
/// Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub enum ReportPhrase {
    /// Command help for `report`.
    #[code("cli-report-about")]
    #[text("Preview an issue report, and file it with the GitHub CLI.")]
    About,
    /// Long command help for `report`.
    #[code("cli-report-long-about")]
    #[text(
        "Preview an issue report for the slopvac repository. The command prints the exact target, title and body, with paths, user and host names and secrets removed as far as it can tell, and warns about text that may still hold a credential. It posts nothing. It prints a prefilled issue-form URL. --body-file writes the body to a file, --gh prints the exact `gh issue create` command for that file, and --gh --yes runs it through the GitHub CLI after you have read the preview. Slopvac never reads a token: `gh` holds the login. Run `slopvac prime report` for the workflow an agent follows, including the user's approval of the exact text."
    )]
    LongAbout,
    /// Help for the category argument.
    #[code("cli-report-category-help")]
    #[text("The kind of report; it picks the issue form.")]
    CategoryHelp,
    /// Help for `--title`.
    #[code("cli-report-title-help")]
    #[text("The issue title, without the category prefix.")]
    TitleHelp,
    /// Help for `--field`.
    #[code("cli-report-field-help")]
    #[text("A form field. Repeat for several fields, or several dropdown options.")]
    FieldHelp,
    /// Help for `--include-text`.
    #[code("cli-report-include-text-help")]
    #[text("Read the document sample for the `input` field from this file.")]
    IncludeTextHelp,
    /// Help for `--body-file`.
    #[code("cli-report-body-file-help")]
    #[text("Write the previewed body to this file. The write replaces the file in one step.")]
    BodyFileHelp,
    /// Help for `--gh`.
    #[code("cli-report-gh-help")]
    #[text("Print the exact `gh issue create` command for the body file.")]
    GhHelp,
    /// Help for `--yes`.
    #[code("cli-report-yes-help")]
    #[text("With --gh: run that command after the preview. Offline mode refuses it.")]
    YesHelp,
    /// Help for `--open`.
    #[code("cli-report-open-help")]
    #[text("Open the prefilled issue form in a browser.")]
    OpenHelp,
    /// Command help for `report consent`.
    #[code("cli-report-consent-about")]
    #[text("Record whether agents may report slopvac problems for you.")]
    ConsentAbout,
    /// Long command help for `report consent`.
    #[code("cli-report-consent-long-about")]
    #[text(
        "Record, for your user account, whether an agent may report slopvac problems. `on` and `off` write the answer to a file in the state directory, and `show` prints it with the time it was recorded. The answer is unset until you record one. The command refuses to change the answer in CI."
    )]
    ConsentLongAbout,
    /// Command help for `report consent on`.
    #[code("cli-report-consent-on-about")]
    #[text("Allow agents to report slopvac problems.")]
    ConsentOnAbout,
    /// Command help for `report consent off`.
    #[code("cli-report-consent-off-about")]
    #[text("Do not allow agents to report slopvac problems.")]
    ConsentOffAbout,
    /// Command help for `report consent show`.
    #[code("cli-report-consent-show-about")]
    #[text("Print the recorded answer and when it was recorded.")]
    ConsentShowAbout,
    /// `--body-file -` was given.
    #[code("cli-report-body-file-stdin")]
    #[text("`-` means standard input to gh; write ./- for a file of that name")]
    BodyFileStdin,
    /// The body file path cannot be printed in a command.
    #[code("cli-report-body-file-not-utf8")]
    #[text("the path must be UTF-8 without control characters, so the command can be printed")]
    BodyFileNotUtf8,
    /// The temporary file for the body cannot be created.
    #[code("cli-report-body-file-create")]
    #[text("cannot create a file there")]
    BodyFileCreate,
    /// The body cannot be written.
    #[code("cli-report-body-file-write")]
    #[text("cannot write the file")]
    BodyFileWrite,
    /// The body file cannot replace the path.
    #[code("cli-report-body-file-replace")]
    #[text("cannot replace that path")]
    BodyFileReplace,
    /// `gh issue create` failed with no exit status.
    #[code("cli-report-gh-timed-out")]
    #[text("it failed or timed out")]
    GhTimedOut,
    /// `gh` was not found, as a short phrase.
    #[code("cli-report-gh-missing-short")]
    #[text("the GitHub CLI `gh` was not found on PATH")]
    GhMissingShort,
    /// `gh` is not logged in, as a short phrase.
    #[code("cli-report-gh-not-logged-in-short")]
    #[text("`gh auth status` failed; run `gh auth login`")]
    GhNotLoggedInShort,
    /// `gh issue create` failed with no exit status, as a short phrase.
    #[code("cli-report-gh-failed-timeout")]
    #[text("`gh issue create` failed or timed out")]
    GhFailedTimeout,
    /// The sample file cannot be opened.
    #[code("cli-report-sample-open")]
    #[text("cannot open the file")]
    SampleOpen,
    /// The sample is not a regular file.
    #[code("cli-report-sample-not-regular")]
    #[text("not a regular file")]
    SampleNotRegular,
    /// The sample cannot be read.
    #[code("cli-report-sample-read")]
    #[text("cannot read the file")]
    SampleRead,
    /// The sample is over the size limit.
    #[code("cli-report-sample-too-large")]
    #[text("the file is larger than 256 KiB")]
    SampleTooLarge,
    /// The sample is not UTF-8.
    #[code("cli-report-sample-not-utf8")]
    #[text("the file is not valid UTF-8")]
    SampleNotUtf8,
    /// A `--field` argument has no `=`.
    #[code("cli-report-invalid-field-syntax")]
    #[text("--field takes ID=VALUE")]
    InvalidFieldSyntax,
    /// The `input` field was given on the command line.
    #[code("cli-report-invalid-input-field")]
    #[text(
        "the `input` field holds document text; pass it with --include-text FILE so the preview shows it"
    )]
    InvalidInputField,
    /// A field or the sample holds a control character.
    #[code("cli-report-invalid-control")]
    #[text("a field or the document text holds a control character such as an escape sequence")]
    InvalidControl,
    /// The title is empty or has several lines.
    #[code("cli-report-invalid-title-line")]
    #[text("--title takes one non-empty line")]
    InvalidTitleLine,
    /// The title holds a control character.
    #[code("cli-report-invalid-title-control")]
    #[text("--title holds a control character")]
    InvalidTitleControl,
    /// A gate finding: a credential.
    #[code("cli-report-kind-credential")]
    #[text("credential")]
    KindCredential,
    /// A gate finding: a repository that is not slopvac.
    #[code("cli-report-kind-repository")]
    #[text("private repository")]
    KindRepository,
    /// A gate finding: an absolute path.
    #[code("cli-report-kind-path")]
    #[text("private path")]
    KindPath,
    /// A gate finding: a name from this machine.
    #[code("cli-report-kind-local-name")]
    #[text("local name")]
    KindLocalName,
    /// The body file was written.
    #[code("cli-report-body-written")]
    #[text("Wrote the body to the file you named.")]
    BodyWritten,
    /// The first line of the footer.
    #[code("cli-report-footer-not-posted")]
    #[text(
        "Nothing was posted. Add --body-file FILE --gh to print the GitHub CLI command, and --yes to run it."
    )]
    FooterNotPosted,
}

/// Text of `slopvac report` with parameters: details of a refusal, warnings and output lines.
#[derive(Debug, Catalog)]
pub enum ReportText {
    /// `gh issue create` exited with a status.
    #[code("cli-report-gh-exit")]
    #[text("exit {code}")]
    GhExit {
        /// The exit status, as digits.
        code: String,
    },
    /// `gh issue create` failed with a status, as a short phrase.
    #[code("cli-report-gh-failed-exit")]
    #[text("`gh issue create` failed (exit {code})")]
    GhFailedExit {
        /// The exit status, as digits.
        code: String,
    },
    /// A `--field` id the category does not have.
    #[code("cli-report-invalid-unknown-field")]
    #[text("unknown field id for `{category}`; its fields are: {known}")]
    InvalidUnknownField {
        /// The category key.
        category: String,
        /// The field ids of the category, comma separated.
        known: String,
    },
    /// A dropdown given more than one option.
    #[code("cli-report-invalid-one-option")]
    #[text("`{id}` takes one option")]
    InvalidOneOption {
        /// The field id.
        id: String,
    },
    /// A dropdown option that is not offered.
    #[code("cli-report-invalid-options")]
    #[text("`{id}` must be one of: {options}")]
    InvalidOptions {
        /// The field id.
        id: String,
        /// The options, separated by a bar.
        options: String,
    },
    /// An input field with several lines.
    #[code("cli-report-invalid-single-line")]
    #[text("`{id}` is a single line")]
    InvalidSingleLine {
        /// The field id.
        id: String,
    },
    /// A field given more than once.
    #[code("cli-report-invalid-repeated")]
    #[text("`{id}` was given more than once")]
    InvalidRepeated {
        /// The field id.
        id: String,
    },
    /// The title is over the limit.
    #[code("cli-report-invalid-title-long")]
    #[text("--title is longer than {max} characters; GitHub caps a title at 256 with its prefix")]
    InvalidTitleLong {
        /// The limit.
        #[count]
        max: usize,
    },
    /// `--include-text` for a category with no document field.
    #[code("cli-report-invalid-no-text-field")]
    #[text("`{category}` has no document text field, so --include-text does not apply")]
    InvalidNoTextField {
        /// The category key.
        category: String,
    },
    /// Text that may hold private data.
    #[code("cli-report-possible")]
    #[text("possible {kind} in field `{field}`: review it before posting")]
    Possible {
        /// What the gate found, from [`ReportPhrase`].
        kind: String,
        /// The field id.
        field: String,
    },
    /// The preview: target, title and body.
    #[code("cli-report-preview")]
    #[text("Target: {repository}\nTitle: {title}\n\n{body}")]
    Preview {
        /// The repository.
        repository: String,
        /// The cleaned title.
        title: String,
        /// The cleaned body.
        body: String,
    },
    /// A warning line.
    #[code("cli-report-warning")]
    #[text("warning: {text}")]
    Warning {
        /// The warning, from [`ReportText::Possible`].
        text: String,
    },
    /// The `gh issue create` command line.
    #[code("cli-report-gh-command")]
    #[text("GitHub CLI command:\n{line}")]
    GhCommand {
        /// The quoted command.
        line: String,
    },
    /// What `gh` printed after it created the issue.
    #[code("cli-report-created")]
    #[text("Created: {url}")]
    Created {
        /// The issue URL, as `gh` printed it.
        url: String,
    },
    /// The prefilled form URL.
    #[code("cli-report-footer-url")]
    #[text("Prefilled issue form: {url}")]
    FooterUrl {
        /// The URL.
        url: String,
    },
    /// The labels the form sets.
    #[code("cli-report-footer-labels")]
    #[text("The issue form adds the labels: {labels}")]
    FooterLabels {
        /// The labels, comma separated.
        labels: String,
    },
    /// Fields the URL could not hold.
    #[code("cli-report-footer-omitted")]
    #[text("The URL is too long to hold these fields; paste them from the preview: {fields}")]
    FooterOmitted {
        /// The field ids, comma separated.
        fields: String,
    },
    /// Standard output cannot be written. The code `output-failed` is spelled where it is raised.
    #[code("cli-report-output-failed")]
    #[text("cannot write output: {kind}")]
    OutputFailed {
        /// The kind of the I/O error.
        kind: String,
    },
}

/// The catalog entries of the fixed phrases, for the message registry.
#[must_use]
pub fn phrase_entries() -> &'static [slopvac_messages::Entry] {
    <ReportPhrase as Catalog>::entries()
}

/// The catalog entries of the text with parameters, for the message registry.
#[must_use]
pub fn text_entries() -> &'static [slopvac_messages::Entry] {
    <ReportText as Catalog>::entries()
}

/// The phrase as owned text.
pub(super) fn phrase(phrase: ReportPhrase) -> String {
    phrase.as_static().to_owned()
}

/// The text of `text`.
pub(super) fn say(text: &ReportText) -> String {
    slopvac_messages::render(text)
}
impl From<ReportError> for crate::errors::CliError {
    /// The code and the text both come from the catalog entry.
    fn from(error: ReportError) -> Self {
        Self::new(error.code(), slopvac_messages::render(&error))
    }
}
