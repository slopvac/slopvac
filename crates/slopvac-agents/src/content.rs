//! Generated text: steering blocks, the skill and the reviewer agent files, and
//! Agent Skills validation (<https://agentskills.io/specification>).

use std::fmt::Write as _;

use crate::harness::{AgentFormat, REVIEWER_NAME, SKILL_NAME};
use crate::managed::MARKER;
use crate::messages::{AgentsPhrase, AgentsText, quoted};

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
    // The skill file is generated content that the harness reads, not a message.
    // ast-grep-ignore: user-text-literal
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
                Err(AgentsText::ModelClaude {
                    model: quoted(model),
                    aliases: CLAUDE_MODEL_ALIASES.join(", "),
                    inherit: INHERIT.to_owned(),
                }
                .to_string())
            }
        }
    }
}

/// Shell rules of the reviewer, in match order: the last matching rule wins, so the
/// catch-all deny comes first. Only read-only subcommands are allowed. `lint` is left out
/// because it can write (`--fix`), and `setup`, `init`, `fix` and `review` (which may call a
/// provider) match no allow rule.
///
/// The shell operators are denied after the allows as a second layer. `OpenCode` splits a
/// compound command into subcommands and checks each one, and it refuses a command whose
/// dynamic evaluation it cannot analyse. Those two behaviours are the first layer; the
/// documentation does not spell out every substitution form, so these rules do not rely on
/// them alone.
///
/// Semantics: <https://opencode.ai/docs/permissions/> (matching, scanner) and
/// <https://opencode.ai/docs/agents/#permissions>.
const SHELL_RULES: [(&str, &str); 12] = [
    ("*", "deny"),
    ("slopvac prime *", "allow"),
    ("slopvac rules *", "allow"),
    ("slopvac explain *", "allow"),
    ("slopvac claims *", "allow"),
    ("*>*", "deny"),
    ("*;*", "deny"),
    ("*&*", "deny"),
    ("*|*", "deny"),
    ("*$(*", "deny"),
    ("*`*", "deny"),
    ("*\n*", "deny"),
];

/// Quote `value` as a YAML double-quoted scalar.
fn yaml_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// The reviewer's permissions in both `OpenCode` frontmatter dialects.
///
/// `OpenCode` reads a file with any of `permissions`, `system`, `request` or `disabled` as the
/// current (V2) format and ignores V1 fields in it; every other file is read as V1. The file
/// carries both fields, so each version enforces the same policy and neither sees the other.
fn opencode_permissions() -> String {
    let mut v2 =
        String::from("permissions:\n  - action: \"*\"\n    resource: \"*\"\n    effect: deny\n");
    for action in ["read", "grep", "glob"] {
        // YAML for the harness, not a message.
        // ast-grep-ignore: user-text-literal
        let _ = write!(
            v2,
            "  - action: {action}\n    resource: \"*\"\n    effect: allow\n"
        );
    }
    for env in ["*.env", "*.env.*"] {
        // ast-grep-ignore: user-text-literal
        let _ = write!(
            v2,
            "  - action: read\n    resource: {}\n    effect: deny\n",
            yaml_quote(env)
        );
    }
    for (resource, effect) in SHELL_RULES {
        // ast-grep-ignore: user-text-literal
        let _ = write!(
            v2,
            "  - action: shell\n    resource: {}\n    effect: {effect}\n",
            yaml_quote(resource)
        );
    }
    let mut v1 = String::from(
        "permission:\n  \"*\": deny\n  edit: deny\n  task: deny\n  webfetch: deny\n  websearch: deny\n  \
         external_directory: deny\n  grep: allow\n  glob: allow\n  read:\n    \"*\": allow\n    \"*.env\": deny\n    \
         \"*.env.*\": deny\n  bash:\n",
    );
    for (resource, effect) in SHELL_RULES {
        let _ = writeln!(v1, "    {}: {effect}", yaml_quote(resource));
    }
    v2 + &v1
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
        // ast-grep-ignore: user-text-literal
        Some(m) => format!("model: {m}\n"),
        None => format!("{MODEL_PLACEHOLDER}\n"),
    };
    let head = match format {
        // The agent file is generated content that the harness reads, not a message.
        // ast-grep-ignore: user-text-literal
        AgentFormat::Opencode => format!(
            "name: {REVIEWER_NAME}\ndescription: \"{description}\"\nmode: subagent\n{model_line}\
             {}",
            opencode_permissions()
        ),
        // The `tools` allowlist cannot scope Bash to commands, so the reviewer gets no Bash.
        // ast-grep-ignore: user-text-literal
        AgentFormat::Claude => format!(
            "name: {REVIEWER_NAME}\ndescription: \"{description}\"\n\
             tools: Read, Grep, Glob\n{model_line}"
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
        return vec![AgentsPhrase::SkillFrontMatterStart.as_static().into()];
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
            problems.push(
                AgentsText::SkillFrontMatterLine {
                    line: line.to_owned(),
                }
                .to_string(),
            );
        }
    }
    if !closed {
        problems.push(AgentsPhrase::SkillFrontMatterUnclosed.as_static().into());
    }
    let get = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
    match get("name") {
        None => problems.push(AgentsPhrase::SkillNameRequired.as_static().into()),
        Some(n) => {
            let ok_chars = n
                .chars()
                .all(|c| c == '-' || (c.is_alphanumeric() && !c.is_uppercase()));
            if n.is_empty() || n.chars().count() > 64 {
                problems.push(AgentsPhrase::SkillNameLength.as_static().into());
            }
            if !ok_chars || n.starts_with('-') || n.ends_with('-') || n.contains("--") {
                problems.push(AgentsPhrase::SkillNameChars.as_static().into());
            }
            if n != dir_name {
                problems.push(
                    AgentsText::SkillNameDirectory {
                        name: quoted(n),
                        directory: quoted(dir_name),
                    }
                    .to_string(),
                );
            }
        }
    }
    match get("description") {
        None => problems.push(AgentsPhrase::SkillDescriptionRequired.as_static().into()),
        Some(d) if d.is_empty() || d.chars().count() > 1024 => {
            problems.push(AgentsPhrase::SkillDescriptionLength.as_static().into());
        }
        Some(_) => {}
    }
    if let Some(c) = get("compatibility")
        && (c.is_empty() || c.chars().count() > 500)
    {
        problems.push(AgentsPhrase::SkillCompatibilityLength.as_static().into());
    }
    for (k, _) in &fields {
        if !SKILL_FIELDS.contains(&k.as_str()) {
            problems.push(AgentsText::SkillFieldUnknown { field: k.clone() }.to_string());
        }
    }
    problems
}
