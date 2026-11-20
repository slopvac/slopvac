//! The generated reviewer agent grants read-only access, checked on its parsed front matter.
//!
//! The evaluator below follows the `OpenCode` documentation:
//! - <https://opencode.ai/docs/permissions/>: the last matching rule wins, no match means
//!   `ask`, `*` matches any run of characters and `?` one character against the whole value,
//!   a pattern ending in ` *` also matches the command without arguments, and a compound
//!   command is checked as its extracted subcommands, where any `deny` denies, otherwise any
//!   `ask` asks.
//! - <https://opencode.ai/docs/agents/#permissions>: the Markdown agent `permissions` list.
//! - <https://opencode.ai/docs/migrate-v1>: a file with `permissions` is read as the current
//!   format and one without as V1 (`permission`, `bash`), so the file carries both.
//!
//! The scanner is not reproducible here, so the test checks the whole command line and
//! every piece split at `;`, `&`, `|` and newlines, and requires all of them to be allowed.

use std::fmt;

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use slopvac_agents::content::reviewer_agent;
use slopvac_agents::harness::AgentFormat;

/// A YAML map that keeps document order, because the last matching rule wins.
#[derive(Debug, Default)]
struct Ordered(Vec<(String, String)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of strings")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = m.next_entry::<String, String>()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Debug, Deserialize)]
struct Rule {
    action: String,
    resource: String,
    effect: String,
}

/// The V1 `permission` map.
#[derive(Debug, Deserialize)]
struct V1Permission {
    #[serde(rename = "*")]
    any: String,
    edit: String,
    task: String,
    webfetch: String,
    websearch: String,
    external_directory: String,
    grep: String,
    glob: String,
    read: Ordered,
    bash: Ordered,
}

#[derive(Debug, Deserialize)]
struct OpencodeAgent {
    mode: String,
    permissions: Vec<Rule>,
    permission: V1Permission,
}

#[derive(Debug, Deserialize)]
struct ClaudeAgent {
    tools: String,
}

fn front_matter(agent: &str) -> &str {
    let rest = agent.strip_prefix("---\n").expect("opens with ---");
    rest.split_once("\n---\n").expect("closes with ---").0
}

/// Whole-value wildcard match: `*` any run, `?` one character.
fn glob(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|i| glob(rest, &text[i..])),
        Some(('?', rest)) => text.split_first().is_some_and(|(_, t)| glob(rest, t)),
        Some((c, rest)) => text
            .split_first()
            .is_some_and(|(t, tail)| t == c && glob(rest, tail)),
    }
}

/// The `OpenCode` pattern match: a trailing ` *` also matches the bare command.
fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob(&p, &t)
        || pattern
            .strip_suffix(" *")
            .is_some_and(|bare| glob(&bare.chars().collect::<Vec<_>>(), &t))
}

/// The last matching rule decides; no match means `ask`.
fn effect(rules: &[Rule], action: &str, resource: &str) -> String {
    rules
        .iter()
        .rev()
        .find(|r| matches(&r.action, action) && matches(&r.resource, resource))
        .map_or_else(|| "ask".to_owned(), |r| r.effect.clone())
}

/// Several resources: any `deny` denies, otherwise any `ask` asks, otherwise allow.
fn combine(effects: &[String]) -> &'static str {
    if effects.iter().any(|e| e == "deny") {
        "deny"
    } else if effects.iter().any(|e| e == "ask") {
        "ask"
    } else {
        "allow"
    }
}

/// The command line and every piece between shell separators.
fn shell_resources(command: &str) -> Vec<String> {
    let mut all = vec![command.to_owned()];
    all.extend(
        command
            .split([';', '&', '|', '\n'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    );
    all
}

fn shell_effect(rules: &[Rule], command: &str) -> &'static str {
    let effects: Vec<String> = shell_resources(command)
        .iter()
        .map(|r| effect(rules, "shell", r))
        .collect();
    combine(&effects)
}

/// The V1 map as the same ordered rule list.
fn v1_rules(p: &V1Permission) -> Vec<Rule> {
    let rule = |action: &str, resource: &str, effect: &str| Rule {
        action: action.to_owned(),
        resource: resource.to_owned(),
        effect: effect.to_owned(),
    };
    let mut rules = vec![
        rule("*", "*", &p.any),
        rule("edit", "*", &p.edit),
        rule("subagent", "*", &p.task),
        rule("webfetch", "*", &p.webfetch),
        rule("websearch", "*", &p.websearch),
        rule("external_directory", "*", &p.external_directory),
        rule("grep", "*", &p.grep),
        rule("glob", "*", &p.glob),
    ];
    rules.extend(p.read.0.iter().map(|(r, e)| rule("read", r, e)));
    rules.extend(p.bash.0.iter().map(|(r, e)| rule("shell", r, e)));
    rules
}

#[test]
fn claude_reviewer_has_read_tools_and_no_shell() {
    let text = reviewer_agent(AgentFormat::Claude, None);
    let agent: ClaudeAgent = serde_saphyr::from_str(front_matter(&text)).unwrap();
    let tools: Vec<&str> = agent.tools.split(',').map(str::trim).collect();
    assert_eq!(tools, ["Read", "Grep", "Glob"]);
}

const ALLOWED: [&str; 8] = [
    "slopvac prime",
    "slopvac prime review",
    "slopvac prime review --type update",
    "slopvac rules",
    "slopvac rules list",
    "slopvac explain some-rule",
    "slopvac claims",
    "slopvac claims doc.md --original old.md",
];

const REFUSED: [&str; 20] = [
    "slopvac init",
    "slopvac setup claude",
    "slopvac fix doc.md",
    "slopvac lint doc.md --fix",
    "slopvac review doc.md",
    "slopvac primer",
    "slopvac prime review > notes.md",
    "slopvac prime review; rm -rf .",
    "slopvac prime review && slopvac setup claude",
    "slopvac prime review | sh",
    "slopvac prime review & slopvac setup claude",
    "slopvac prime review\nslopvac setup claude",
    "slopvac prime review\nrm -rf .",
    "slopvac prime $(rm -rf .)",
    "slopvac prime `id`",
    "slopvac prime \"$(id)\"",
    "rm -rf .",
    "git push",
    "cat .env",
    "sh -c 'slopvac prime review'",
];

#[test]
fn opencode_reviewer_edits_nothing_and_runs_only_read_only_slopvac_commands() {
    let text = reviewer_agent(AgentFormat::Opencode, None);
    let agent: OpencodeAgent = serde_saphyr::from_str(front_matter(&text)).unwrap();
    assert_eq!(agent.mode, "subagent");
    // Both dialects must give the same answer to every question.
    let v1 = v1_rules(&agent.permission);
    for (name, rules) in [("permissions", &agent.permissions), ("permission", &v1)] {
        for action in [
            "edit",
            "subagent",
            "webfetch",
            "websearch",
            "external_directory",
            "execute",
        ] {
            assert_eq!(
                effect(rules, action, "anything"),
                "deny",
                "{name}: {action}"
            );
        }
        for action in ["read", "grep", "glob"] {
            assert_eq!(
                effect(rules, action, "src/lib.rs"),
                "allow",
                "{name}: {action}"
            );
        }
        for secret in [".env", "app/prod.env", "app/.env.local"] {
            assert_eq!(effect(rules, "read", secret), "deny", "{name}: {secret}");
        }
        for command in ALLOWED {
            assert_eq!(shell_effect(rules, command), "allow", "{name}: {command:?}");
        }
        for command in REFUSED {
            assert_eq!(shell_effect(rules, command), "deny", "{name}: {command:?}");
        }
    }
}
