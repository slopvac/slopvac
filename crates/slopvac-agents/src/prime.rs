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
use crate::guidance::{Guidance, Report, Review, ReviewDepth, Write};
use crate::harness;
use crate::messages::{AgentsPhrase, AgentsText, quoted};
use crate::models::{self, Evals};
use crate::review::{Approval, ReviewType};
use crate::setup::{EXIT_FINDINGS, EXIT_INCOMPLETE, EXIT_OK};

/// Schema version the topics name for lint JSON and config.
pub const SCHEMA_VERSION: u8 = 3;

/// The base topic; it has no name on the command line.
const BASE: &str = include_str!("../topics/base.md.j2");

/// Topics in index order: name and source.
const TOPICS: [(&str, &str); 8] = [
    ("write", include_str!("../topics/write.md.j2")),
    ("review", include_str!("../topics/review.md.j2")),
    ("report", include_str!("../topics/report.md.j2")),
    ("diagrams", include_str!("../topics/diagrams.md.j2")),
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
    /// The `[guidance]` controls. `level` here wins over `guidance.level`.
    pub guidance: Guidance,
    /// Detail level.
    pub level: Level,
    /// Render the review topic for one type; `None` shows how to pick.
    pub review_type: Option<ReviewType>,
    /// `review_approval` from the config.
    pub review_approval: Approval,
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
            let mut usage = AgentsText::PrimeCommandUsage {
                name: c.get_name().to_owned(),
            }
            .to_string();
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
            .map_err(|e| {
                AgentsError::usage(AgentsText::PrimeSyntaxFailed {
                    detail: e.to_string(),
                })
            })?,
    );
    let commands: Vec<Value> = command_lines(cli)
        .into_iter()
        .map(|c| context! { usage => c.usage, about => c.about })
        .collect();
    // A topic whose control is "off" leaves the index; the index line is the pointer to each
    // enabled step, so the base guide stays within its word budget.
    let topics: Vec<Value> = TOPICS
        .iter()
        .filter(|(n, _)| settings.guidance.topic_enabled(n))
        .map(|(n, s)| {
            let ask = *n == "review" && settings.guidance.review == Review::Ask;
            context! { name => *n, summary => split_summary(s).0, ask => ask }
        })
        .collect();
    let ctx = context! {
        version => env!("CARGO_PKG_VERSION"),
        level => settings.level.name(),
        exit_ok => EXIT_OK,
        exit_findings => EXIT_FINDINGS,
        exit_incomplete => EXIT_INCOMPLETE,
        schema_version => SCHEMA_VERSION,
        harnesses => harness::names(),
        model_guidance => models::prime_guidance(&Evals::embedded()?, settings.review_type),
        migrate_guide => slopvac_migrate::prime_migrate(),
        review_type => settings.review_type.map_or("auto", ReviewType::name),
        write => settings.guidance.write == Write::Auto,
        review => match settings.guidance.review {
            Review::Auto => "auto",
            Review::Ask => "ask",
            Review::Off => "off",
        },
        review_depth => match settings.guidance.review_depth {
            ReviewDepth::SelfReview => "self",
            ReviewDepth::Subagent => "subagent",
            ReviewDepth::Adversary => "adversary",
        },
        report => settings.guidance.report == Report::Ask,
        review_approval => if settings.review_approval == Approval::Auto { "auto" } else { "ask" },
        commands => commands,
        topics => topics,
    };
    let body = split_summary(source).1.replace("\r\n", "\n");
    env.render_str(&body, ctx).map_err(|e| {
        AgentsError::usage(AgentsText::PrimeTopicRenderFailed {
            topic: quoted(name),
            detail: e.to_string(),
        })
    })
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
    if let Some(notice) = settings.guidance.disabled_notice(name) {
        return Ok(notice);
    }
    let source = if name.is_empty() {
        BASE
    } else {
        TOPICS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| *s)
            .ok_or_else(|| {
                AgentsError::usage(AgentsText::PrimeTopicUnknown {
                    topic: quoted(name),
                    choices: topic_names().join(", "),
                })
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
    let mut problems = Vec::new();
    let names = std::iter::once(None).chain(topic_names().into_iter().map(Some));
    let variants = [
        None,
        Some(ReviewType::Mechanical),
        Some(ReviewType::Update),
        Some(ReviewType::Rewrite),
    ];
    for topic in names {
        for review_type in variants {
            let settings = Settings {
                level: Level::Full,
                review_type,
                ..Settings::default()
            };
            let Ok(text) = render(topic, &settings, cli, Path::new("/nonexistent")) else {
                problems.push(
                    AgentsText::PrimeCheckNotRendered {
                        topic: quoted(topic),
                    }
                    .to_string(),
                );
                continue;
            };
            for span in text.split('`').skip(1).step_by(2) {
                problems.extend(check_span(span, cli, topic));
            }
        }
    }
    problems.sort();
    problems.dedup();
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
        return vec![
            AgentsText::PrimeCheckUnknownCommand {
                at: at.to_owned(),
                span: span.to_owned(),
                command: quoted(first),
            }
            .to_string(),
        ];
    };
    words
        .filter(|w| w.starts_with("--"))
        .filter(|w| {
            let flag = w.trim_end_matches(['.', ',']);
            let flag = flag.trim_start_matches("--");
            !sub.get_arguments().any(|a| a.get_long() == Some(flag))
                && !cli.get_arguments().any(|a| a.get_long() == Some(flag))
        })
        .map(|w| {
            AgentsText::PrimeCheckUnknownFlag {
                at: at.to_owned(),
                span: span.to_owned(),
                flag: w.to_owned(),
            }
            .to_string()
        })
        .collect()
}

/// The `prime` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    Command::new("prime")
        .about(AgentsPhrase::PrimeAbout.as_static())
        .arg(
            Arg::new("topic")
                .value_name("TOPIC")
                .value_parser(clap::builder::PossibleValuesParser::new(topic_names()))
                .help(AgentsPhrase::PrimeTopic.as_static()),
        )
        .arg(
            Arg::new("level")
                .long("level")
                .value_parser(["minimal", "default", "full"])
                .help(AgentsPhrase::PrimeLevel.as_static()),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .value_parser(["mechanical", "update", "rewrite"])
                .help(AgentsPhrase::PrimeType.as_static()),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_name("FILE")
                .value_parser(clap::value_parser!(PathBuf))
                .help(AgentsPhrase::PrimeConfig.as_static()),
        )
}

/// Run the parsed `prime` command. `cli` is the root command, so the base
/// topic lists its subcommands. `root` is the project directory.
pub fn run(matches: &ArgMatches, cli: &Command, root: &Path) -> Result<String, AgentsError> {
    let config = matches.get_one::<PathBuf>("config").map(PathBuf::as_path);
    let guidance = Guidance::load(root, config)?;
    let level = match matches.get_one::<String>("level").map(String::as_str) {
        Some("minimal") => Level::Minimal,
        Some("full") => Level::Full,
        Some(_) => Level::Default,
        None => guidance.level,
    };
    let settings = Settings {
        level,
        guidance,
        review_type: matches
            .get_one::<String>("type")
            .and_then(|t| ReviewType::parse(t)),
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
