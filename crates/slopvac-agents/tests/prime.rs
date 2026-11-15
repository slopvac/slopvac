//! Behaviour tests for `slopvac prime` through its public interface.

use std::fs;

use clap::{Arg, ArgAction, Command};
use slopvac_agents::prime::{self, Level, Settings};

fn flags(name: &'static str, about: &'static str, flags: &[&'static str]) -> Command {
    flags.iter().fold(Command::new(name).about(about), |c, f| {
        c.arg(Arg::new(*f).long(*f).action(ArgAction::SetTrue))
    })
}

/// A command tree with the shape of the 3.0 CLI, using the real `setup` and `prime`.
fn tree() -> Command {
    Command::new("slopvac")
        .subcommand(
            flags(
                "lint",
                "Check prose files",
                &[
                    "format",
                    "diff-base",
                    "mode",
                    "fix",
                    "explain-config",
                    "stdin",
                ],
            )
            .arg(Arg::new("targets").num_args(1..)),
        )
        .subcommand(flags(
            "rules",
            "List rules",
            &["kind", "format", "include-excluded", "category", "genre"],
        ))
        .subcommand(
            flags("explain", "Explain one rule", &["format"])
                .arg(Arg::new("rule_id").required(true)),
        )
        .subcommand(flags(
            "report",
            "Preview or post an issue",
            &["submit", "include-text"],
        ))
        .subcommand(slopvac_migrate::command())
        .subcommand(slopvac_agents::claims::command())
        .subcommand(slopvac_agents::review::command())
        .subcommand(Command::new("doctor").about("Check the installation"))
        .subcommand(
            Command::new("plugins")
                .about("Manage packs and plugins")
                .subcommand(Command::new("trust")),
        )
        .subcommand(Command::new("secret").about("Internal").hide(true))
        .subcommand(slopvac_agents::setup::command())
        .subcommand(prime::command())
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn render(topic: Option<&str>, level: Level) -> String {
    prime::render(
        topic,
        &Settings {
            level,
            ..Settings::default()
        },
        &tree(),
        std::path::Path::new("/nonexistent"),
    )
    .unwrap()
}

#[test]
fn base_topic_fits_its_word_budget_at_every_level() {
    let minimal = words(&render(None, Level::Minimal));
    let default = words(&render(None, Level::Default));
    let full = words(&render(None, Level::Full));
    assert!(default <= 260, "default base is {default} words");
    assert!(
        minimal < default && default < full,
        "{minimal} {default} {full}"
    );
}

#[test]
fn every_topic_fits_its_budget() {
    for topic in prime::topic_names() {
        let n = words(&render(Some(topic), Level::Default));
        let budget = match topic {
            "review" => 360,
            "migrate" => 700,
            _ => 330,
        };
        assert!(n <= budget, "{topic} is {n} words");
    }
}

#[test]
fn command_list_matches_the_clap_tree() {
    let text = render(None, Level::Default);
    let listed: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("- `slopvac "))
        .filter_map(|l| l.split(['`', ' ']).next())
        .filter(|name| tree().find_subcommand(name).is_some())
        .collect();
    assert_eq!(
        listed,
        [
            "lint", "rules", "explain", "report", "migrate", "claims", "review", "doctor",
            "plugins", "setup", "prime"
        ]
    );
    assert!(text.contains("`slopvac lint [TARGETS]...`: Check prose files"));
    assert!(text.contains("`slopvac explain <RULE_ID>`"));
}

#[test]
fn topic_index_lists_each_topic_with_its_summary() {
    let text = render(None, Level::Default);
    for topic in prime::topic_names() {
        assert!(text.contains(&format!("- `{topic}`: ")), "{topic}");
    }
    assert!(text.contains("- `migrate`: Move a 2.x project to 3.0"));
    assert!(!render(None, Level::Minimal).contains("`migrate`:"));
}

#[test]
fn full_level_adds_text_and_minimal_drops_it() {
    assert!(render(Some("write"), Level::Full).contains("Scope of a run"));
    assert!(!render(Some("write"), Level::Default).contains("Scope of a run"));
    assert!(!render(Some("write"), Level::Minimal).contains("## Sentences"));
}

#[test]
fn topics_name_only_commands_and_flags_that_exist() {
    assert_eq!(prime::check_topics(&tree()), Vec::<String>::new());
    let without_migrate = Command::new("slopvac").subcommand(Command::new("lint"));
    let problems = prime::check_topics(&without_migrate);
    assert!(
        problems.iter().any(|p| p.contains("migrate")),
        "{problems:?}"
    );
}

#[test]
fn topics_state_the_contract_exit_codes() {
    let text = render(None, Level::Default);
    assert!(
        text.contains(
            "Exit 0 passes. Exit 1 means a gate failed. Exit 2 means the run is incomplete"
        )
    );
}

#[test]
fn unknown_topic_is_a_usage_error_that_lists_the_topics() {
    let err = prime::render(
        Some("nope"),
        &Settings::default(),
        &tree(),
        std::path::Path::new("."),
    )
    .unwrap_err();
    assert_eq!(err.code(), "setup-usage");
    assert!(err.to_string().contains("write, review, report"));
}

#[test]
fn prime_md_without_markers_replaces_the_base_topic_only() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".slopvac")).unwrap();
    fs::write(dir.path().join(".slopvac/PRIME.md"), "House rules.\n").unwrap();
    let settings = Settings::default();
    let base = prime::render(None, &settings, &tree(), dir.path()).unwrap();
    let write = prime::render(Some("write"), &settings, &tree(), dir.path()).unwrap();
    assert_eq!(base, "House rules.\n");
    assert!(write.starts_with("# Prime: write"));
}

#[test]
fn prime_md_topic_markers_replace_named_topics() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".slopvac")).unwrap();
    fs::write(
        dir.path().join(".slopvac/PRIME.md"),
        "<!-- slopvac:topic write -->\nWrite ours.\n<!-- slopvac:topic rules -->\nRules ours {{ not_a_template }}\n",
    )
    .unwrap();
    let settings = Settings::default();
    let render = |t| prime::render(t, &settings, &tree(), dir.path()).unwrap();
    assert_eq!(render(Some("write")), "Write ours.\n");
    assert_eq!(render(Some("rules")), "Rules ours {{ not_a_template }}\n");
    assert!(render(None).starts_with("# Slopvac agent guidance"));
}

#[test]
fn run_renders_the_requested_topic_and_level() {
    let m = prime::command()
        .try_get_matches_from(["prime", "setup", "--level", "full"])
        .unwrap();
    let out = prime::run(&m, &tree(), std::path::Path::new("/nonexistent")).unwrap();
    assert!(out.starts_with("# Prime: setup"));
    assert!(out.contains("agents, codex, omp, kiro, claude, opencode"));
    assert!(
        prime::command()
            .try_get_matches_from(["prime", "bogus"])
            .is_err()
    );
}

#[test]
fn prime_migrate_prints_the_migration_guide_from_its_one_source() {
    let text = render(Some("migrate"), Level::Default);
    assert_eq!(text.trim_end(), slopvac_migrate::prime_migrate().trim_end());
    assert!(slopvac_migrate::guide_doc().contains(slopvac_migrate::prime_migrate()));
    for level in [Level::Minimal, Level::Full] {
        assert_eq!(
            render(Some("migrate"), level),
            text,
            "the guide has one level"
        );
    }
}

#[test]
fn diagrams_topic_is_indexed_and_names_every_supported_format() {
    assert!(render(None, Level::Default).contains("- `diagrams`: "));
    let text = render(Some("diagrams"), Level::Default);
    assert!(text.starts_with("# Prime: diagrams"));
    for needle in [
        "experimental",
        "slopvac lint <paths>",
        ".mmd",
        ".puml",
        ".drawio",
        ".drawio.svg",
        ".drawio.png",
        ".excalidraw",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
}
