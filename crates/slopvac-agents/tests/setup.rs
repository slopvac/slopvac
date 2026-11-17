//! Behaviour tests for `slopvac setup` through its public interface.

use std::fs;
use std::path::Path;

use slopvac_agents::content::{self, BlockKind};
use slopvac_agents::harness::{AgentFormat, SKILL_NAME};
use slopvac_agents::setup::{
    EXIT_FINDINGS, EXIT_INCOMPLETE, EXIT_OK, Options, command, execute,
    install_keeping_unrequested, options,
};

fn opts(root: &Path, harness: &str) -> Options {
    Options {
        harness: Some(harness.into()),
        list: false,
        remove: false,
        check: false,
        root: root.into(),
        block: BlockKind::Standard,
        no_skill: false,
        agent: false,
        reviewer_model: None,
    }
}

fn check(root: &Path, harness: &str) -> u8 {
    execute(&Options {
        check: true,
        ..opts(root, harness)
    })
    .unwrap()
    .exit
}

#[test]
fn install_then_check_passes_for_every_harness() {
    for h in slopvac_agents::harness::names() {
        let dir = tempfile::tempdir().unwrap();
        execute(&opts(dir.path(), h)).unwrap();
        assert_eq!(check(dir.path(), h), EXIT_OK, "{h}");
    }
}

#[test]
fn check_on_empty_project_is_findings() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(check(dir.path(), "claude"), EXIT_FINDINGS);
}

#[test]
fn install_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    execute(&opts(dir.path(), "opencode")).unwrap();
    let again = execute(&opts(dir.path(), "opencode")).unwrap();
    assert!(
        again.lines.iter().all(|l| l.starts_with("unchanged")),
        "{:?}",
        again.lines
    );
}

#[test]
fn install_keeps_surrounding_text_and_remove_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("AGENTS.md");
    fs::write(&file, "# Mine\n\nKeep this.\n").unwrap();
    execute(&opts(dir.path(), "codex")).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("# Mine\n\nKeep this.\n\n<!-- slopvac:begin -->"));
    execute(&Options {
        remove: true,
        ..opts(dir.path(), "codex")
    })
    .unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "# Mine\n\nKeep this.\n");
    assert!(!dir.path().join(".agents/skills").join(SKILL_NAME).exists());
}

#[test]
fn markers_inside_a_fenced_example_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("CLAUDE.md");
    fs::write(&file, "```md\n<!-- slopvac:begin -->\n```\n").unwrap();
    execute(&opts(dir.path(), "claude")).unwrap();
    assert_eq!(check(dir.path(), "claude"), EXIT_OK);
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .starts_with("```md\n<!-- slopvac:begin -->\n```\n")
    );
}

#[test]
fn half_a_marker_pair_is_malformed_and_check_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("CLAUDE.md"), "<!-- slopvac:begin -->\n").unwrap();
    assert_eq!(check(dir.path(), "claude"), EXIT_INCOMPLETE);
    let err = execute(&opts(dir.path(), "claude")).unwrap_err();
    assert_eq!(err.code(), "steering-malformed");
}

#[test]
fn stale_block_is_findings_and_install_repairs_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("CLAUDE.md"),
        "<!-- slopvac:begin -->\nold\n<!-- slopvac:end -->\n",
    )
    .unwrap();
    assert_eq!(check(dir.path(), "claude"), EXIT_FINDINGS);
    execute(&opts(dir.path(), "claude")).unwrap();
    assert_eq!(check(dir.path(), "claude"), EXIT_OK);
}

#[test]
fn block_size_changes_the_text_and_check_follows_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        block: BlockKind::Minimal,
        ..opts(dir.path(), "agents")
    })
    .unwrap();
    assert_eq!(check(dir.path(), "agents"), EXIT_FINDINGS);
    let min = execute(&Options {
        block: BlockKind::Minimal,
        check: true,
        ..opts(dir.path(), "agents")
    })
    .unwrap();
    assert_eq!(min.exit, EXIT_OK);
}

#[test]
fn handwritten_skill_is_never_overwritten_or_removed() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir
        .path()
        .join(format!(".claude/skills/{SKILL_NAME}/SKILL.md"));
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(
        &skill,
        "---\nname: slopvac-prose\ndescription: mine\n---\nmine\n",
    )
    .unwrap();
    assert_eq!(
        execute(&opts(dir.path(), "claude")).unwrap_err().code(),
        "steering-not-managed"
    );
    assert_eq!(check(dir.path(), "claude"), EXIT_INCOMPLETE);
    assert!(fs::read_to_string(&skill).unwrap().contains("mine\n"));
}

#[test]
fn no_skill_installs_no_skill_and_check_agrees() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        no_skill: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    assert!(!dir.path().join(".claude/skills").exists());
    let r = execute(&Options {
        no_skill: true,
        check: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    assert_eq!(r.exit, EXIT_OK);
}

#[test]
fn opencode_reviewer_agent_carries_the_model_and_is_subagent_only() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        agent: true,
        reviewer_model: Some("anthropic/claude-sonnet-5-5#high".into()),
        ..opts(dir.path(), "opencode")
    })
    .unwrap();
    let text = fs::read_to_string(dir.path().join(".opencode/agents/slopvac-reviewer.md")).unwrap();
    assert!(text.contains("mode: subagent\nmodel: anthropic/claude-sonnet-5-5#high\n"));
    assert!(text.contains("slopvac prime review"));
    let r = execute(&Options {
        agent: true,
        check: true,
        ..opts(dir.path(), "opencode")
    })
    .unwrap();
    assert_eq!(r.exit, EXIT_FINDINGS, "model differs, so the file is stale");
}

#[test]
fn remove_deletes_the_agent_and_leaves_user_files_in_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        agent: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    let other = dir.path().join(".claude/agents/mine.md");
    fs::write(&other, "mine").unwrap();
    execute(&Options {
        remove: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    assert!(
        !dir.path()
            .join(".claude/agents/slopvac-reviewer.md")
            .exists()
    );
    assert!(other.exists());
}

#[test]
fn usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        Options {
            agent: true,
            ..opts(dir.path(), "kiro")
        },
        Options {
            reviewer_model: Some("m".into()),
            ..opts(dir.path(), "claude")
        },
        Options {
            agent: true,
            reviewer_model: Some("a b\nc".into()),
            ..opts(dir.path(), "claude")
        },
        Options {
            check: true,
            remove: true,
            ..opts(dir.path(), "claude")
        },
        Options {
            harness: None,
            ..opts(dir.path(), "claude")
        },
        Options {
            harness: Some("nope".into()),
            ..opts(dir.path(), "claude")
        },
    ];
    for c in cases {
        assert_eq!(execute(&c).unwrap_err().code(), "setup-usage", "{c:?}");
    }
}

#[test]
fn kiro_new_file_gets_inclusion_front_matter() {
    let dir = tempfile::tempdir().unwrap();
    execute(&opts(dir.path(), "kiro")).unwrap();
    let text = fs::read_to_string(dir.path().join(".kiro/steering/slopvac.md")).unwrap();
    assert!(text.starts_with("---\ninclusion: always\n---\n\n<!-- slopvac:begin -->"));
}

#[test]
fn codex_prefers_a_non_empty_override_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("AGENTS.override.md"), "x\n").unwrap();
    execute(&opts(dir.path(), "codex")).unwrap();
    assert!(
        fs::read_to_string(dir.path().join("AGENTS.override.md"))
            .unwrap()
            .contains("slopvac:begin")
    );
    assert!(!dir.path().join("AGENTS.md").exists());
}

#[cfg(unix)]
#[test]
fn symlink_that_leaves_the_project_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".claude")).unwrap();
    let err = execute(&opts(dir.path(), "claude")).unwrap_err();
    assert_eq!(err.code(), "steering-unsafe-target");
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}

#[test]
fn list_names_every_harness() {
    let dir = tempfile::tempdir().unwrap();
    let r = execute(&Options {
        list: true,
        harness: None,
        ..opts(dir.path(), "x")
    })
    .unwrap();
    assert_eq!(r.lines.len(), 6);
    assert!(
        r.lines
            .iter()
            .any(|l| l.starts_with("opencode") && l.replace('\\', "/").contains(".opencode/agents"))
    );
}

#[test]
fn generated_skill_follows_the_agent_skills_spec_and_bad_ones_do_not() {
    assert_eq!(
        content::validate_skill(SKILL_NAME, &content::skill_stub()),
        Vec::<String>::new()
    );
    let bad = [
        ("a", "---\nname: Bad\ndescription: d\n---\n"),
        ("a", "---\nname: a--b\ndescription: d\n---\n"),
        ("a", "---\nname: a\n---\n"),
        ("a", "---\nname: b\ndescription: d\n---\n"),
        ("a", "---\nname: a\ndescription: d\nextra: 1\n---\n"),
    ];
    for (dir, text) in bad {
        assert!(!content::validate_skill(dir, text).is_empty(), "{text}");
    }
}

#[test]
fn command_parses_the_documented_flags() {
    let m = command()
        .try_get_matches_from([
            "setup",
            "opencode",
            "--agent",
            "--reviewer-model",
            "p/m",
            "--block",
            "minimal",
            "--no-skill",
        ])
        .unwrap();
    let o = options(&m);
    assert!(o.agent && o.no_skill);
    assert_eq!(o.block, BlockKind::Minimal);
    assert_eq!(o.reviewer_model.as_deref(), Some("p/m"));
    assert!(
        command()
            .try_get_matches_from(["setup", "opencode", "--reviewer-model", "m"])
            .is_err()
    );
}

#[test]
fn reviewer_agent_formats_differ_by_harness() {
    assert!(content::reviewer_agent(AgentFormat::Claude, None).contains("tools: Read"));
    assert!(content::reviewer_agent(AgentFormat::Opencode, None).contains("mode: subagent"));
}

fn managed_files(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let h = slopvac_agents::harness::find("claude").unwrap();
    (
        root.join(h.skill_path().unwrap().unwrap()),
        root.join(h.agent_path().unwrap().unwrap()),
    )
}

#[test]
fn keeping_install_leaves_an_existing_reviewer_and_skill_alone() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        agent: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    let (skill, reviewer) = managed_files(dir.path());
    assert!(skill.is_file() && reviewer.is_file());
    let r = install_keeping_unrequested(&Options {
        no_skill: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    assert!(skill.is_file() && reviewer.is_file(), "files were removed");
    assert_eq!(
        r.lines.len(),
        1,
        "only the steering block is reported: {:?}",
        r.lines
    );
}

#[test]
fn execute_still_removes_what_the_options_leave_out() {
    let dir = tempfile::tempdir().unwrap();
    execute(&Options {
        agent: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    execute(&Options {
        no_skill: true,
        ..opts(dir.path(), "claude")
    })
    .unwrap();
    let (skill, reviewer) = managed_files(dir.path());
    assert!(!skill.exists() && !reviewer.exists());
}

#[test]
fn keeping_install_refuses_list_remove_and_check_and_changes_nothing() {
    let flags: [fn(&mut Options); 3] = [|o| o.list = true, |o| o.remove = true, |o| o.check = true];
    for set in flags {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "mine\n").unwrap();
        let mut o = opts(dir.path(), "claude");
        set(&mut o);
        let err = install_keeping_unrequested(&o).unwrap_err();
        assert!(
            matches!(err, slopvac_agents::AgentsError::Usage(_)),
            "{err:?}"
        );
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["CLAUDE.md"], "the project changed");
        assert_eq!(
            fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap(),
            "mine\n"
        );
    }
}
const PLACEHOLDER: &str = "# model: <a model from a different family than the author's>";

fn agent_opts(root: &Path, harness: &str, model: Option<&str>) -> Options {
    Options {
        agent: true,
        reviewer_model: model.map(Into::into),
        ..opts(root, harness)
    }
}

#[test]
fn an_unset_or_inherited_reviewer_model_writes_a_placeholder_and_warns() {
    for harness in ["claude", "opencode"] {
        for model in [None, Some("inherit")] {
            let dir = tempfile::tempdir().unwrap();
            let out = execute(&agent_opts(dir.path(), harness, model)).unwrap();
            assert_eq!(out.exit, EXIT_OK, "{harness} {model:?}");
            let codes: Vec<&str> = out.notices.iter().map(|n| n.code).collect();
            assert_eq!(codes, ["setup-reviewer-model-unset"], "{harness} {model:?}");
            let text = fs::read_to_string(
                dir.path()
                    .join(format!(".{harness}/agents/slopvac-reviewer.md")),
            )
            .unwrap();
            assert!(text.contains(&format!("{PLACEHOLDER}\n")), "{harness}");
            assert!(!text.contains("\nmodel:"), "{harness}: no live model line");
        }
    }
}

#[test]
fn a_set_reviewer_model_gives_no_warning() {
    let dir = tempfile::tempdir().unwrap();
    let out = execute(&agent_opts(dir.path(), "claude", Some("opus"))).unwrap();
    assert_eq!(out.notices, vec![]);
}

#[test]
fn installing_without_the_agent_gives_no_model_warning() {
    let dir = tempfile::tempdir().unwrap();
    let out = execute(&opts(dir.path(), "claude")).unwrap();
    assert_eq!(out.notices, vec![]);
}

#[test]
fn check_reports_an_unset_model_as_a_warning_not_as_stale() {
    let dir = tempfile::tempdir().unwrap();
    execute(&agent_opts(dir.path(), "claude", None)).unwrap();
    let out = execute(&Options {
        check: true,
        ..agent_opts(dir.path(), "claude", None)
    })
    .unwrap();
    assert_eq!(out.exit, EXIT_OK);
    assert!(
        out.lines.iter().all(|l| l.starts_with("current")),
        "{:?}",
        out.lines
    );
    assert_eq!(out.notices.len(), 1);
    assert_eq!(out.notices[0].code, "setup-reviewer-model-unset");
}

#[test]
fn claude_accepts_an_alias_a_claude_id_or_inherit() {
    for model in [
        "sonnet",
        "opus",
        "haiku",
        "fable",
        "claude-opus-5-5",
        "inherit",
    ] {
        let dir = tempfile::tempdir().unwrap();
        execute(&agent_opts(dir.path(), "claude", Some(model)))
            .unwrap_or_else(|e| panic!("{model}: {e}"));
    }
}

#[test]
fn claude_refuses_any_other_model_with_the_format_code() {
    for model in [
        "gpt-5",
        "anthropic/claude-sonnet-5-5",
        "sonet",
        "claude-",
        "Sonnet",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let err = execute(&agent_opts(dir.path(), "claude", Some(model))).unwrap_err();
        assert_eq!(err.code(), "setup-reviewer-model-format", "{model}");
        assert!(
            !dir.path().join(".claude").exists(),
            "{model}: nothing written"
        );
    }
}

#[test]
fn opencode_keeps_provider_model_and_variant() {
    let dir = tempfile::tempdir().unwrap();
    let out = execute(&agent_opts(
        dir.path(),
        "opencode",
        Some("openai/gpt-5#high"),
    ))
    .unwrap();
    assert_eq!(out.notices, vec![]);
}
