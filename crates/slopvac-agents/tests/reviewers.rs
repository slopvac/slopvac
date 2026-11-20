//! Behaviour tests for finding installed reviewer agent files.

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_agents::content::{self, BlockKind};
use slopvac_agents::harness::AgentFormat;
use slopvac_agents::reviewers::{ReviewerAgent, reviewer_agents};
use slopvac_agents::setup::{Options, execute};

fn install(root: &Path, harness: &str, model: Option<&str>) {
    execute(&Options {
        harness: Some(harness.into()),
        list: false,
        remove: false,
        check: false,
        root: root.into(),
        block: BlockKind::Standard,
        no_skill: true,
        agent: true,
        reviewer_model: model.map(Into::into),
    })
    .unwrap();
}

#[test]
fn an_empty_project_has_no_reviewer_agents() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(reviewer_agents(dir.path()).unwrap(), Vec::new());
}

#[test]
fn each_installed_agent_reports_its_harness_path_and_model() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path(), "claude", Some("opus"));
    install(dir.path(), "opencode", Some("openai/gpt-5#high"));
    assert_eq!(
        reviewer_agents(dir.path()).unwrap(),
        [
            ReviewerAgent {
                harness: "claude",
                path: PathBuf::from(".claude/agents/slopvac-reviewer.md"),
                model: Some("opus".into()),
            },
            ReviewerAgent {
                harness: "opencode",
                path: PathBuf::from(".opencode/agents/slopvac-reviewer.md"),
                model: Some("openai/gpt-5#high".into()),
            },
        ]
    );
}

#[test]
fn a_placeholder_or_inherit_model_reads_as_unset() {
    for model in [None, Some("inherit")] {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "claude", model);
        let found = reviewer_agents(dir.path()).unwrap();
        assert_eq!(found.len(), 1, "{model:?}");
        assert_eq!(found[0].model, None, "{model:?}");
    }
}

#[test]
fn a_file_slopvac_did_not_write_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".claude/agents/slopvac-reviewer.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "---\nname: mine\nmodel: opus\n---\nbody\n").unwrap();
    assert_eq!(reviewer_agents(dir.path()).unwrap(), Vec::new());
}
fn seed(root: &Path, front_matter_model: &str) {
    let path = root.join(".claude/agents/slopvac-reviewer.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = content::reviewer_agent(AgentFormat::Claude, Some(front_matter_model));
    fs::write(path, text).unwrap();
}

#[test]
fn a_live_inherit_model_line_reads_as_unset() {
    for raw in [
        "inherit",
        "'inherit'",
        "\"inherit\"",
        "inherit # follow the session",
    ] {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), raw);
        let found = reviewer_agents(dir.path()).unwrap();
        assert_eq!(found.len(), 1, "{raw}");
        assert_eq!(found[0].model, None, "{raw}");
    }
}

#[test]
fn a_model_line_is_read_as_a_yaml_scalar() {
    let cases = [
        ("'claude-opus-5-5'", "claude-opus-5-5"),
        ("\"openai/gpt-5#high\"", "openai/gpt-5#high"),
        ("opus # the strongest", "opus"),
        ("openai/gpt-5", "openai/gpt-5"),
    ];
    for (raw, want) in cases {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), raw);
        let found = reviewer_agents(dir.path()).unwrap();
        assert_eq!(found[0].model.as_deref(), Some(want), "{raw}");
    }
}
