//! The issue-report field definitions: the single source for `slopvac report` and for the
//! GitHub issue forms in `.github/ISSUE_TEMPLATE`.
//!
//! Ported from `slopvac.reporting` in the Python reference, with the help texts the forms carry.
//! `tests/report_forms.rs` regenerates the forms from these definitions and fails on any drift.

use super::{Category, Field, Kind};

/// The note at the top of every form.
pub const INTRO: &str = "An AI agent filing a report MUST disclose itself: tick Reported by an AI agent and fill in Agent, Model, and Harness and version. Reports must not contain paths or file names. Do not include user names, host names, repository names, organization names, or other private data. For security vulnerabilities, use the private vulnerability report on the Security tab.";

/// Fields every category starts with. Proposals (`new-rule`, `rule-improvement`) do not require
/// the environment and command.
fn common(proposal: bool, command_description: &'static str) -> Vec<Field> {
    vec![
        Field::new("reporter", "Reporter", Kind::Checkboxes).checks(&[("Reported by an AI agent", false)]),
        Field::new("agent", "Agent", Kind::Input).describe("Agent product name, for example Claude Code, Codex, omp, Kiro, or Cursor. Write N/A when unknown."),
        Field::new("model", "Model", Kind::Input).describe("Record the active model name exactly as the harness reports it in its system prompt, context, or /model selection. Do not use self-description or config defaults. Write N/A when unavailable."),
        Field::new("harness", "Harness and version", Kind::Input).describe("Use the harness's version command, for example claude --version, codex --version, or omp --version. Write N/A when unavailable."),
        Field::new("version", "slopvac version", Kind::Input).describe("Output of `slopvac --version`."),
        Field::new("latest", "Reproduces on the latest release", Kind::Dropdown).describe("Rerun the command with `uvx --from slopvac@latest slopvac ...`. If the latest release fixes it, upgrade instead of filing.").options(&["Yes", "Not checked"]),
        Field::new("duplicates", "Duplicate search", Kind::Checkboxes).checks(&[("I searched existing issues for duplicates", true)]),
        with_required(Field::new("environment", "Environment", Kind::Textarea).render("json").describe("Output of `slopvac doctor --format json`, which contains no paths."), !proposal),
        with_required(Field::new("command", "Command", Kind::Textarea).render("shell").describe(command_description), !proposal),
    ]
}

fn with_required(field: Field, required: bool) -> Field {
    if required { field } else { field.optional() }
}

/// The default description of the command field.
const COMMAND: &str = "The exact command or commands you ran. Pipe the sample into `slopvac lint --stdin`. Do not include paths or file names.";

/// Every report category, in the order of the Python registry.
#[must_use]
#[allow(clippy::too_many_lines)] // One literal table: splitting it would hide the forms.
pub fn categories() -> Vec<Category> {
    vec![
        Category {
            key: "bug",
            form_file: "bug.yml",
            form_name: "Bug report",
            description: "Report a crash, incorrect result, or broken behavior.",
            title_prefix: "[bug]: ",
            labels: &["bug", "needs-triage"],
            meaning: "Crash, traceback, wrong exit code, wrong or unstable output format, config/setup/init/Vale/cache/prime defects.",
            fields: {
                let mut fields = common(false, COMMAND);
                fields.extend([
                    Field::new("what-happened", "What happened", Kind::Textarea),
                    Field::new("expected", "Expected behavior", Kind::Textarea),
                    Field::new("output", "Output", Kind::Textarea)
                        .optional()
                        .render("text")
                        .describe("stderr, traceback, or report excerpt"),
                    Field::new("config", "Configuration", Kind::Textarea)
                        .optional()
                        .render("toml")
                        .describe("Relevant `slopvac.toml` lines."),
                ]);
                fields
            },
        },
        Category {
            key: "false-positive",
            form_file: "false-positive.yml",
            form_name: "Rule flags text that is not slop",
            description: "Report a rule flagging text that is not slop.",
            title_prefix: "[false positive]: ",
            labels: &["false-positive", "needs-triage"],
            meaning: "A rule flags text that is not slop.",
            fields: {
                let mut fields = common(false, COMMAND);
                fields.extend([
                    Field::new("rule-id", "Rule ID", Kind::Input),
                    Field::new("context", "Genre and profile", Kind::Input).describe("Genre: the genre name from the matching row of the Genres and profiles table in `slopvac prime`. Profile: the profile your command ran with, from `--profile` or your config."),
                    Field::new("input", "Flagged text", Kind::Textarea).render("markdown").describe("The smallest sample that still triggers the finding. Paste the sample text itself. A local file path is not enough."),
                    Field::new("finding", "Finding", Kind::Textarea).render("json").describe("The finding object from `slopvac lint --stdin --format json`. Its path reads `<stdin>`."),
                    Field::new("why", "Why this is not slop", Kind::Textarea),
                ]);
                fields
            },
        },
        Category {
            key: "missed-slop",
            form_file: "missed-slop.yml",
            form_name: "Slop that slopvac missed",
            description: "Report slop that no finding reports.",
            title_prefix: "[missed slop]: ",
            labels: &["false-negative", "needs-triage"],
            meaning: "Slop that no finding reports, where an existing rule should cover it; when unsure, use this form.",
            fields: {
                let mut fields = common(false, COMMAND);
                fields.extend([
                    Field::new("context", "Genre and profile", Kind::Input).describe("Genre: the genre name from the matching row of the Genres and profiles table in `slopvac prime`. Profile: the profile your command ran with, from `--profile` or your config."),
                    Field::new("input", "Missed text", Kind::Textarea).render("markdown").describe("Paste the sample text itself. A local file path is not enough."),
                    Field::new("expected-rule", "Rule that should fire", Kind::Input).optional().describe("Leave empty when you do not know. When no rule covers the pattern, use the new rule form."),
                    Field::new("why", "Why this is slop", Kind::Textarea),
                ]);
                fields
            },
        },
        Category {
            key: "mis-tagged",
            form_file: "mis-tagged.yml",
            form_name: "Slop tagged incorrectly",
            description: "Report a finding with the wrong rule, severity, or span.",
            title_prefix: "[mis-tagged]: ",
            labels: &["mis-tagged", "needs-triage"],
            meaning: "The flagged text is slop, but the rule, severity, or span/location is wrong.",
            fields: {
                let mut fields = common(false, COMMAND);
                fields.extend([
                    Field::new("rule-id", "Rule ID", Kind::Input),
                    Field::new("context", "Genre and profile", Kind::Input).describe("Genre: the genre name from the matching row of the Genres and profiles table in `slopvac prime`. Profile: the profile your command ran with, from `--profile` or your config."),
                    Field::new("input", "Flagged text", Kind::Textarea).render("markdown").describe("Paste the sample text itself. A local file path is not enough."),
                    Field::new("finding", "Finding", Kind::Textarea).render("json").describe("The finding object from `slopvac lint --stdin --format json`. Its path reads `<stdin>`."),
                    Field::new("problem", "What is wrong", Kind::Dropdown).options(&["Wrong rule", "Wrong severity", "Wrong span or location"]).multiple(),
                    Field::new("expected", "Expected finding", Kind::Textarea).describe("The rule, severity, or span you expected."),
                ]);
                fields
            },
        },
        Category {
            key: "wrong-fix",
            form_file: "wrong-fix.yml",
            form_name: "Correct match, wrong suggestion or fix",
            description: "Report a correct finding with a wrong suggestion, fix, message, or guidance.",
            title_prefix: "[wrong fix]: ",
            labels: &["wrong-fix", "needs-triage"],
            meaning: "The flagged text is slop and the right rule fired, but the message, suggested replacement, fix guidance, or `--fix` edit is wrong, unclear, or changes the meaning.",
            fields: {
                let mut fields = common(
                    false,
                    "The exact command or commands you ran. Pipe the sample into `slopvac lint --stdin`, or run `--fix` on a synthetic `sample.md`. Do not include other paths or file names.",
                );
                fields.extend([
                    Field::new("rule-id", "Rule ID", Kind::Input),
                    Field::new("context", "Genre and profile", Kind::Input).describe("Genre: the genre name from the matching row of the Genres and profiles table in `slopvac prime`. Profile: the profile your command ran with, from `--profile` or your config."),
                    Field::new("input", "Flagged text", Kind::Textarea).render("markdown").describe("Paste the sample text itself. A local file path is not enough."),
                    Field::new("finding", "Finding", Kind::Textarea).render("json").describe("The finding object from `slopvac lint --stdin --format json`. Its path reads `<stdin>`."),
                    Field::new("problem", "What is wrong", Kind::Dropdown).options(&["Wrong suggested replacement", "--fix changed the meaning", "--fix produced broken text", "Misleading or unclear message", "Fix guidance does not apply here"]).multiple(),
                    Field::new("fixed-text", "Text after the fix", Kind::Textarea).optional().render("markdown").describe("Text after `slopvac lint sample.md --fix`, run on a synthetic sample in an empty temporary directory."),
                    Field::new("expected", "Expected suggestion", Kind::Textarea),
                ]);
                fields
            },
        },
        Category {
            key: "new-rule",
            form_file: "new-rule.yml",
            form_name: "New rule proposal",
            description: "Propose a writing pattern that no existing rule covers.",
            title_prefix: "[new rule]: ",
            labels: &["new-rule", "needs-triage"],
            meaning: "A writing pattern that no existing rule covers.",
            fields: {
                let mut fields = common(true, COMMAND);
                fields.extend([
                    Field::new("pattern", "Pattern", Kind::Textarea)
                        .describe("Describe the writing pattern and why it reads as slop."),
                    Field::new("bad-examples", "Examples that should fire", Kind::Textarea)
                        .render("markdown"),
                    Field::new(
                        "good-examples",
                        "Examples that must not fire",
                        Kind::Textarea,
                    )
                    .render("markdown"),
                    Field::new("kind", "Rule kind", Kind::Dropdown).options(&[
                        "Pattern (deterministic)",
                        "Judgement (reviewer question)",
                        "Unsure",
                    ]),
                    Field::new("genres", "Genres", Kind::Dropdown)
                        .optional()
                        .options(&[
                            "consumer",
                            "reference",
                            "change-comms",
                            "internal",
                            "informal",
                        ])
                        .multiple(),
                    Field::new("related", "Related existing rules", Kind::Input).optional(),
                    Field::new("source", "Evidence or source", Kind::Textarea).optional(),
                ]);
                fields
            },
        },
        Category {
            key: "rule-improvement",
            form_file: "rule-improvement.yml",
            form_name: "Rule improvement",
            description: "Propose a change to an existing rule as a whole.",
            title_prefix: "[rule improvement]: ",
            labels: &["rule-improvement", "needs-triage"],
            meaning: "A change to an existing rule as a whole: accuracy across many findings, examples, exceptions, judgement question, tier or severity, genre recommendation, message or fix wording in general, or docs.",
            fields: {
                let mut fields = common(true, COMMAND);
                fields.extend([
                    Field::new("rule-id", "Rule ID", Kind::Input),
                    Field::new("aspect", "What to improve", Kind::Dropdown).options(&["Accuracy across many findings", "Message or fix wording", "Examples", "Exceptions", "Judgement question", "Severity or tier", "Genre recommendation", "Documentation"]).multiple(),
                    Field::new("current", "Current behavior", Kind::Textarea),
                    Field::new("proposal", "Proposed change", Kind::Textarea),
                    Field::new("accuracy", "Accuracy sample", Kind::Textarea).optional().describe("Findings judged on a sample: how many were true slop, how many were not, and the corpus they came from."),
                    Field::new("evidence", "Evidence", Kind::Textarea).optional(),
                ]);
                fields
            },
        },
    ]
}
