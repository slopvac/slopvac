//! Generated text: steering blocks, the skill and the reviewer agent files, and
//! Agent Skills validation (<https://agentskills.io/specification>).

use crate::harness::{AgentFormat, REVIEWER_NAME, SKILL_NAME};
use crate::managed::MARKER;

const SKILL_FIELDS: [&str; 6] = [
    "name",
    "description",
    "license",
    "compatibility",
    "metadata",
    "allowed-tools",
];

/// Size of the steering block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockKind {
    /// Two lines: run prime.
    Minimal,
    /// The prime pointer plus the exit-code rule.
    #[default]
    Standard,
}

/// The managed steering block.
#[must_use]
pub fn steering_block(kind: BlockKind) -> String {
    let body = match kind {
        BlockKind::Minimal => {
            "Before writing or reviewing prose, run `slopvac prime` and follow it.\n"
        }
        BlockKind::Standard => {
            "Before writing or reviewing prose, run `slopvac prime` and follow the current\n\
             CLI guidance. Use `slopvac rules` and `slopvac explain <rule_id>` for rule\n\
             details and named exceptions.\n\
             \n\
             Treat lint exit 2 as incomplete.\n"
        }
    };
    format!("<!-- slopvac:begin -->\n## Slopvac\n\n{body}<!-- slopvac:end -->\n")
}

/// The skill stub. It carries triggers only; the guidance comes from `slopvac prime`.
#[must_use]
pub fn skill_stub() -> String {
    format!(
        "---\n\
         name: {SKILL_NAME}\n\
         description: >-\n  \
         Use when writing, editing, or reviewing prose: README files, documentation,\n  \
         Markdown pages, PR or MR descriptions, commit messages, release notes, and\n  \
         specifications. Run before drafting such text and again before handing it\n  \
         off, even when the user does not mention slopvac.\n\
         ---\n\
         {MARKER}\n\
         \n\
         # Slopvac prose\n\
         \n\
         Before drafting or editing a README, documentation, a Markdown page, a PR or\n\
         MR description, a commit message, release notes, or a specification, run\n\
         `slopvac prime write` and follow it.\n\
         \n\
         Before handing that prose off, run `slopvac prime review` and follow it.\n"
    )
}

/// A reviewer model id is a provider/model string with an optional variant.
#[must_use]
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/#@-".contains(c))
}

/// Model aliases a Claude Code subagent accepts in its `model:` field
/// (<https://code.claude.com/docs/en/sub-agents#choose-a-model>).
pub const CLAUDE_MODEL_ALIASES: [&str; 4] = ["sonnet", "opus", "haiku", "fable"];

/// The value that asks a harness to use the parent session's model.
pub const INHERIT: &str = "inherit";

/// The commented placeholder written when no reviewer model is set.
pub const MODEL_PLACEHOLDER: &str = "# model: <a model from a different family than the author's>";

/// Check a reviewer model against the dialect of the agent file.
///
/// Claude Code takes an alias, a `claude-*` model ID or `inherit`. `OpenCode` takes
/// `provider/model` with an optional `#variant` and has no `inherit`; the caller
/// treats `inherit` as unset before it gets here.
///
/// # Errors
/// A message that names the accepted forms.
pub fn check_model_format(format: AgentFormat, model: &str) -> Result<(), String> {
    match format {
        AgentFormat::Opencode => Ok(()),
        AgentFormat::Claude => {
            if model == INHERIT
                || CLAUDE_MODEL_ALIASES.contains(&model)
                || model
                    .strip_prefix("claude-")
                    .is_some_and(|rest| !rest.is_empty())
            {
                Ok(())
            } else {
                Err(format!(
                    "reviewer model {model:?} is not valid for claude; use an alias ({}), a claude-* model ID or {INHERIT}.",
                    CLAUDE_MODEL_ALIASES.join(", ")
                ))
            }
        }
    }
}

/// The reviewer agent file. It runs only when `slopvac prime review` invokes it.
///
/// With no model, the file carries [`MODEL_PLACEHOLDER`] as a comment, so the
/// harness falls back to the parent session's model.
#[must_use]
pub fn reviewer_agent(format: AgentFormat, model: Option<&str>) -> String {
    let description = "Reviews prose for slopvac. Launch only when `slopvac prime review` \
                       tells you to; never for other tasks.";
    let body = "Run the review pass that `slopvac prime review` describes for the document \
                you receive. Do not edit files. Report each finding as rule id, quoted text, \
                and proposed rewrite, then list the facts of the received document and mark \
                each kept or wrong.";
    let model_line = match model {
        Some(m) => format!("model: {m}\n"),
        None => format!("{MODEL_PLACEHOLDER}\n"),
    };
    let head = match format {
        AgentFormat::Opencode => format!(
            "name: {REVIEWER_NAME}\ndescription: \"{description}\"\nmode: subagent\n{model_line}\
             permissions:\n  - action: \"*\"\n    resource: \"*\"\n    effect: deny\n  \
             - action: read\n    resource: \"*\"\n    effect: allow\n  \
             - action: grep\n    resource: \"*\"\n    effect: allow\n  \
             - action: glob\n    resource: \"*\"\n    effect: allow\n  \
             - action: shell\n    resource: \"slopvac *\"\n    effect: allow\n"
        ),
        AgentFormat::Claude => format!(
            "name: {REVIEWER_NAME}\ndescription: \"{description}\"\n\
             tools: Read, Grep, Glob, Bash\n{model_line}"
        ),
    };
    format!("---\n{head}---\n{MARKER}\n\n{body}\n")
}

/// Check a `SKILL.md` against the Agent Skills specification. `dir_name` is the
/// skill directory. Returns every problem found.
#[must_use]
pub fn validate_skill(dir_name: &str, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return vec!["front matter must start with ---".into()];
    }
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, v)) = fields.last_mut() {
                if !v.is_empty() {
                    v.push(' ');
                }
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            let v = if matches!(v, ">" | ">-" | "|" | "|-") {
                ""
            } else {
                v
            };
            fields.push((k.trim().to_owned(), v.trim_matches('"').to_owned()));
        } else {
            problems.push(format!("front matter line is not key: value: {line}"));
        }
    }
    if !closed {
        problems.push("front matter has no closing ---".into());
    }
    let get = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
    match get("name") {
        None => problems.push("name is required".into()),
        Some(n) => {
            let ok_chars = n
                .chars()
                .all(|c| c == '-' || (c.is_alphanumeric() && !c.is_uppercase()));
            if n.is_empty() || n.chars().count() > 64 {
                problems.push("name must be 1-64 characters".into());
            }
            if !ok_chars || n.starts_with('-') || n.ends_with('-') || n.contains("--") {
                problems.push(
                    "name takes lowercase letters, digits and single hyphens, none at the ends"
                        .into(),
                );
            }
            if n != dir_name {
                problems.push(format!("name {n:?} must match directory {dir_name:?}"));
            }
        }
    }
    match get("description") {
        None => problems.push("description is required".into()),
        Some(d) if d.is_empty() || d.chars().count() > 1024 => {
            problems.push("description must be 1-1024 characters".into());
        }
        Some(_) => {}
    }
    if let Some(c) = get("compatibility")
        && (c.is_empty() || c.chars().count() > 500)
    {
        problems.push("compatibility must be 1-500 characters".into());
    }
    for (k, _) in &fields {
        if !SKILL_FIELDS.contains(&k.as_str()) {
            problems.push(format!("unknown front matter field: {k}"));
        }
    }
    problems
}
