//! The harness registry: where each agent harness reads steering, skills and agents.
//!
//! Sources: claude <https://code.claude.com/docs/en/skills>, codex
//! <https://developers.openai.com/codex/skills>, kiro <https://kiro.dev/docs/skills>,
//! opencode <https://opencode.ai/docs/agents/>, Agent Skills
//! <https://agentskills.io/specification>.

use std::path::PathBuf;

use crate::error::AgentsError;
use crate::fsx::{Project, target};

/// Name of the skill.
pub const SKILL_NAME: &str = "slopvac-prose";
/// Name of the reviewer agent.
pub const REVIEWER_NAME: &str = "slopvac-reviewer";

/// The reviewer agent file dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentFormat {
    /// `.opencode/agents/<name>.md`.
    Opencode,
    /// `.claude/agents/<name>.md`.
    Claude,
}

/// A supported agent harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Harness {
    /// The name used on the command line.
    pub name: &'static str,
    steering: &'static str,
    skills: Option<&'static str>,
    agents: Option<(&'static str, AgentFormat)>,
}

impl Harness {
    /// The reviewer agent file dialect, if the harness has agent files.
    #[must_use]
    pub fn agent_format(&self) -> Option<AgentFormat> {
        self.agents.map(|(_, f)| f)
    }

    /// The steering file, relative to the project. Codex prefers a non-empty
    /// `AGENTS.override.md`.
    pub fn steering_path(&self, project: &Project) -> Result<PathBuf, AgentsError> {
        if self.name == "codex" {
            let over = target("AGENTS.override.md")?;
            if project
                .read(&over)?
                .is_some_and(|b| !b.trim_ascii().is_empty())
            {
                return Ok(over);
            }
        }
        target(self.steering)
    }

    /// The skill file, if the harness loads project skills.
    pub fn skill_path(&self) -> Result<Option<PathBuf>, AgentsError> {
        self.skills
            .map(|d| target(&format!("{d}/{SKILL_NAME}/SKILL.md")))
            .transpose()
    }

    /// The reviewer agent file, if the harness has agent files.
    pub fn agent_path(&self) -> Result<Option<PathBuf>, AgentsError> {
        self.agents
            .map(|(d, _)| target(&format!("{d}/{REVIEWER_NAME}.md")))
            .transpose()
    }

    /// Whether another harness reads the same skill directory.
    #[must_use]
    pub fn shares_skills_with(&self, other: &Harness) -> bool {
        self.name != other.name && self.skills.is_some() && self.skills == other.skills
    }
}

/// Every supported harness, in listing order. `.agents/skills` is the shared
/// skill location that agents, codex and opencode all read.
pub const HARNESSES: [Harness; 6] = [
    Harness {
        name: "agents",
        steering: "AGENTS.md",
        skills: Some(".agents/skills"),
        agents: None,
    },
    Harness {
        name: "codex",
        steering: "AGENTS.md",
        skills: Some(".agents/skills"),
        agents: None,
    },
    Harness {
        name: "omp",
        steering: ".omp/AGENTS.md",
        skills: Some(".omp/skills"),
        agents: None,
    },
    Harness {
        name: "kiro",
        steering: ".kiro/steering/slopvac.md",
        skills: Some(".kiro/skills"),
        agents: None,
    },
    Harness {
        name: "claude",
        steering: "CLAUDE.md",
        skills: Some(".claude/skills"),
        agents: Some((".claude/agents", AgentFormat::Claude)),
    },
    Harness {
        name: "opencode",
        steering: "AGENTS.md",
        skills: Some(".agents/skills"),
        agents: Some((".opencode/agents", AgentFormat::Opencode)),
    },
];

/// Look up a harness by name.
#[must_use]
pub fn find(name: &str) -> Option<Harness> {
    HARNESSES.iter().copied().find(|h| h.name == name)
}

/// The harness names, in order.
#[must_use]
pub fn names() -> Vec<&'static str> {
    HARNESSES.iter().map(|h| h.name).collect()
}
