//! `slopvac prime`: guidance for agents, rendered from Markdown topics.
//!
//! Each topic is a Markdown file under `topics/`, embedded at build time. A topic
//! may open with `<!-- summary: ... -->`, which feeds the index in the base topic.
//! The base topic's command list and topic index come from the clap tree and the
//! registry, so they cannot drift from the code. `.slopvac/PRIME.md` overrides topics.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use minijinja::syntax::SyntaxConfig;
use minijinja::{Environment, UndefinedBehavior, Value, context};

use crate::error::AgentsError;
use crate::harness;
use crate::setup::{EXIT_FINDINGS, EXIT_INCOMPLETE, EXIT_OK};

/// Schema version the topics name for lint JSON and config.
pub const SCHEMA_VERSION: u8 = 3;

/// The base topic; it has no name on the command line.
const BASE: &str = include_str!("../topics/base.md.j2");

/// Topics in index order: name and source.
const TOPICS: [(&str, &str); 6] = [
    ("write", include_str!("../topics/write.md.j2")),
    ("report", include_str!("../topics/report.md.j2")),
    ("configure", include_str!("../topics/configure.md.j2")),
    ("migrate", include_str!("../topics/migrate.md.j2")),
    ("setup", include_str!("../topics/setup.md.j2")),
    ("rules", include_str!("../topics/rules.md.j2")),
];

/// How much detail a topic renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Names and checklists only.
    Minimal,
    /// The standard text.
    #[default]
    Default,
    /// Everything.
    Full,
}

impl Level {
    /// The name used in config and on the command line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Default => "default",
            Self::Full => "full",
        }
    }
}

/// Guidance settings for one render.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Detail level.
    pub level: Level,
    /// The migration guide for `prime migrate`; `None` uses the built-in short text.
    // STUB(slopvac-8l4.11.9): the CLI passes slopvac_migrate::prime_migrate() here.
    pub migrate_guide: Option<String>,
}

/// The topic names, in index order.
#[must_use]
pub fn topic_names() -> Vec<&'static str> {
    TOPICS.iter().map(|(n, _)| *n).collect()
}

fn split_summary(source: &str) -> (&str, &str) {
    let Some(first) = source.lines().next() else {
        return ("", source);
    };
    match first
        .strip_prefix("<!-- summary:")
        .and_then(|s| s.strip_suffix("-->"))
    {
        Some(summary) => (
            summary.trim(),
            source[first.len()..].trim_start_matches(['\r', '\n']),
        ),
        None => ("", source),
    }
}

struct CommandLine {
    usage: String,
    about: String,
}

fn positional(arg: &Arg) -> String {
    let name = arg.get_id().as_str().to_uppercase();
    let many = arg.get_num_args().is_some_and(|r| r.max_values() > 1);
    let dots = if many { "..." } else { "" };
    if arg.is_required_set() {
        format!("<{name}>{dots}")
    } else {
        format!("[{name}]{dots}")
    }
}

fn command_lines(cli: &Command) -> Vec<CommandLine> {
    cli.get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .map(|c| {
            let mut usage = format!("slopvac {}", c.get_name());
            for arg in c.get_positionals() {
                usage.push(' ');
                usage.push_str(&positional(arg));
            }
            let about = c.get_about().map(ToString::to_string).unwrap_or_default();
            CommandLine {
                usage,
                about: about.trim_end_matches('.').to_owned(),
            }
        })
        .collect()
}

/// Parse `.slopvac/PRIME.md` into `(topic, text)` overrides. A file without
/// topic lines overrides the base topic (`""`).
fn overrides(text: &str) -> Vec<(String, String)> {
    const MARK: &str = "<!-- slopvac:topic ";
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(name) = line
            .trim()
            .strip_prefix(MARK)
            .and_then(|s| s.strip_suffix("-->"))
        {
            out.push((name.trim().to_owned(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
        } else if !line.trim().is_empty() {
            out.push((String::new(), line.to_owned()));
        }
    }
    out
}

fn override_for(root: &Path, topic: &str) -> Result<Option<String>, AgentsError> {
    let path = root.join(".slopvac").join("PRIME.md");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| AgentsError::io(&path, e))?;
    Ok(overrides(&text)
        .into_iter()
        .find(|(n, _)| n == topic)
        .map(|(_, body)| body))
}

fn render_template(
    name: &str,
    source: &str,
    settings: &Settings,
    cli: &Command,
) -> Result<String, AgentsError> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_syntax(
        SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .keep_trailing_newline(true)
            .build()
            .map_err(|e| AgentsError::Usage(format!("prime syntax: {e}")))?,
    );
    let commands: Vec<Value> = command_lines(cli)
        .into_iter()
        .map(|c| context! { usage => c.usage, about => c.about })
        .collect();
    let topics: Vec<Value> = TOPICS
        .iter()
        .map(|(n, s)| context! { name => *n, summary => split_summary(s).0 })
        .collect();
    let ctx = context! {
        version => env!("CARGO_PKG_VERSION"),
        level => settings.level.name(),
        exit_ok => EXIT_OK,
        exit_findings => EXIT_FINDINGS,
        exit_incomplete => EXIT_INCOMPLETE,
        schema_version => SCHEMA_VERSION,
        harnesses => harness::names(),
        migrate_guide => settings.migrate_guide.clone().unwrap_or_default(),
        commands => commands,
        topics => topics,
    };
    let body = split_summary(source).1.replace("\r\n", "\n");
    env.render_str(&body, ctx)
        .map_err(|e| AgentsError::Usage(format!("prime topic {name:?} failed to render: {e}")))
}

/// Render a topic (`None` is the base topic) for `cli`, applying any override
/// under `root`.
pub fn render(
    topic: Option<&str>,
    settings: &Settings,
    cli: &Command,
    root: &Path,
) -> Result<String, AgentsError> {
    let name = topic.unwrap_or("");
    let source = if name.is_empty() {
        BASE
    } else {
        TOPICS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| *s)
            .ok_or_else(|| {
                AgentsError::Usage(format!(
                    "unknown prime topic {name:?}; choose from: {}",
                    topic_names().join(", ")
                ))
            })?
    };
    if let Some(text) = override_for(root, name)? {
        return Ok(text);
    }
    let mut out = render_template(name, source, settings, cli)?;
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    Ok(out)
}

/// Check that every `slopvac <command>` and `--flag` that a topic names in a
/// code span exists in `cli`. Returns one message per problem. The CLI crate
/// runs this against the real tree.
#[must_use]
pub fn check_topics(cli: &Command) -> Vec<String> {
    let settings = Settings {
        level: Level::Full,
        ..Settings::default()
    };
    let mut problems = Vec::new();
    let names = std::iter::once(None).chain(topic_names().into_iter().map(Some));
    for topic in names {
        let Ok(text) = render(topic, &settings, cli, Path::new("/nonexistent")) else {
            problems.push(format!("topic {topic:?} does not render"));
            continue;
        };
        for span in text.split('`').skip(1).step_by(2) {
            problems.extend(check_span(span, cli, topic));
        }
    }
    problems
}

fn check_span(span: &str, cli: &Command, topic: Option<&str>) -> Vec<String> {
    let mut words = span.split_whitespace();
    if words.next() != Some("slopvac") {
        return Vec::new();
    }
    let Some(first) = words.next() else {
        return Vec::new();
    };
    let at = topic.unwrap_or("base");
    if first.starts_with('-') {
        return Vec::new();
    }
    let Some(sub) = cli.find_subcommand(first) else {
        return vec![format!("{at}: `{span}` names no command {first:?}")];
    };
    words
        .filter(|w| w.starts_with("--"))
        .filter(|w| {
            let flag = w.trim_end_matches(['.', ',']);
            let flag = flag.trim_start_matches("--");
            !sub.get_arguments().any(|a| a.get_long() == Some(flag))
                && !cli.get_arguments().any(|a| a.get_long() == Some(flag))
        })
        .map(|w| format!("{at}: `{span}` uses unknown flag {w}"))
        .collect()
}

/// The `prime` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    Command::new("prime")
        .about("Print guidance for agents, one topic at a time")
        .arg(
            Arg::new("topic")
                .value_name("TOPIC")
                .value_parser(clap::builder::PossibleValuesParser::new(topic_names()))
                .help("A topic; omit it for the base guide"),
        )
        .arg(
            Arg::new("level")
                .long("level")
                .value_parser(["minimal", "default", "full"])
                .help("Detail level for this run"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_name("FILE")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Config file that sets guidance"),
        )
}

/// Run the parsed `prime` command. `cli` is the root command, so the base
/// topic lists its subcommands. `root` is the project directory.
pub fn run(matches: &ArgMatches, cli: &Command, root: &Path) -> Result<String, AgentsError> {
    let level = match matches.get_one::<String>("level").map(String::as_str) {
        Some("minimal") => Level::Minimal,
        Some("full") => Level::Full,
        _ => Level::Default,
    };
    // STUB(slopvac-8l4.11.6): read `[guidance] level` from the config file once slopvac-config exists.
    let settings = Settings {
        level,
        ..Settings::default()
    };
    let mut out = render(
        matches.get_one::<String>("topic").map(String::as_str),
        &settings,
        cli,
        root,
    )?;
    if !out.ends_with('\n') {
        let _ = writeln!(out);
    }
    Ok(out)
}
