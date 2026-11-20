//! `slopvac setup`: install, check, list or remove harness steering.

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command, builder::PossibleValuesParser};
use slopvac_model::{Notice, NoticeLevel};

use crate::block::{self, State};
use crate::content::{self, BlockKind};
use crate::error::{AgentsError, notice};
use crate::fsx::Project;
use crate::guidance::Guidance;
use crate::harness::{self, AgentFormat, Harness};
use crate::managed;
use crate::messages::{AgentsMessage, AgentsPhrase, AgentsText, quoted};

/// Exit code: pass.
pub const EXIT_OK: u8 = 0;
/// Exit code: a check found missing or stale files.
pub const EXIT_FINDINGS: u8 = 1;
/// Exit code: a file is malformed, so the check is incomplete.
pub const EXIT_INCOMPLETE: u8 = 2;

/// What one setup invocation asks for.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools, reason = "one bool per CLI flag")]
pub struct Options {
    /// The harness, required unless `list`.
    pub harness: Option<String>,
    /// Print the files of each harness.
    pub list: bool,
    /// Remove what setup wrote.
    pub remove: bool,
    /// Report state without writing.
    pub check: bool,
    /// Project directory.
    pub root: PathBuf,
    /// Steering block size.
    pub block: BlockKind,
    /// Skip the skill.
    pub no_skill: bool,
    /// Install the reviewer agent file.
    pub agent: bool,
    /// Model written into the reviewer agent file.
    pub reviewer_model: Option<String>,
}

/// Printed lines and the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// One line per file.
    pub lines: Vec<String>,
    /// Exit code.
    pub exit: u8,
    /// Warnings that leave the exit code alone.
    pub notices: Vec<Notice>,
}

/// The `setup` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    Command::new("setup")
        .about(AgentsPhrase::SetupAbout.as_static())
        .arg(
            Arg::new("harness")
                .value_name("HARNESS")
                .help(AgentsPhrase::SetupHarness.as_static())
                .value_parser(PossibleValuesParser::new(harness::names())),
        )
        .arg(
            Arg::new("list")
                .long("list")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::SetupList.as_static()),
        )
        .arg(
            Arg::new("remove")
                .long("remove")
                .action(ArgAction::SetTrue)
                .conflicts_with("check")
                .help(AgentsPhrase::SetupRemove.as_static()),
        )
        .arg(
            Arg::new("check")
                .long("check")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::SetupCheck.as_static()),
        )
        .arg(
            Arg::new("root")
                .long("root")
                .value_name("DIR")
                .default_value(".")
                .value_parser(clap::value_parser!(PathBuf))
                .help(AgentsPhrase::SetupRoot.as_static()),
        )
        .arg(
            Arg::new("block")
                .long("block")
                .value_parser(["minimal", "standard"])
                .default_value("standard")
                .help(AgentsPhrase::SetupBlock.as_static()),
        )
        .arg(
            Arg::new("no-skill")
                .long("no-skill")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::SetupNoSkill.as_static()),
        )
        .arg(
            Arg::new("agent")
                .long("agent")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::SetupAgent.as_static()),
        )
        .arg(
            Arg::new("reviewer-model")
                .long("reviewer-model")
                .value_name("MODEL")
                .requires("agent")
                .help(AgentsPhrase::SetupReviewerModel.as_static()),
        )
}

/// Read [`Options`] from parsed arguments.
#[must_use]
pub fn options(matches: &ArgMatches) -> Options {
    Options {
        harness: matches.get_one::<String>("harness").cloned(),
        list: matches.get_flag("list"),
        remove: matches.get_flag("remove"),
        check: matches.get_flag("check"),
        root: matches
            .get_one::<PathBuf>("root")
            .cloned()
            .unwrap_or_else(|| ".".into()),
        block: match matches.get_one::<String>("block").map(String::as_str) {
            Some("minimal") => BlockKind::Minimal,
            _ => BlockKind::Standard,
        },
        no_skill: matches.get_flag("no-skill"),
        agent: matches.get_flag("agent"),
        reviewer_model: matches.get_one::<String>("reviewer-model").cloned(),
    }
}

/// Run the parsed `setup` command.
pub fn run(matches: &ArgMatches) -> Result<Outcome, AgentsError> {
    let mut opts = options(matches);
    // `--list` and `--remove` need no config, so a broken one must not stop them.
    // `--no-skill` wins; otherwise the skill goes when write and review are both off.
    let needs_config = !opts.list && !opts.remove && !opts.no_skill;
    if needs_config && !Guidance::load(&opts.root, None)?.skill_wanted() {
        opts.no_skill = true;
    }
    execute(&opts)
}

fn usage(message: impl ToString) -> AgentsError {
    AgentsError::usage(message)
}

struct Plan {
    project: Project,
    harness: Harness,
    steering: PathBuf,
    skill: Option<PathBuf>,
    agent: Option<PathBuf>,
    block_text: String,
    skill_text: Option<String>,
    agent_text: Option<String>,
    /// The reviewer agent is wanted but its model is unset.
    reviewer_model_unset: bool,
}

type Paths = (PathBuf, Option<PathBuf>, Option<PathBuf>);

fn paths(h: &Harness, project: &Project) -> Result<Paths, AgentsError> {
    Ok((h.steering_path(project)?, h.skill_path()?, h.agent_path()?))
}

fn list(project: &Project) -> Result<Outcome, AgentsError> {
    let mut lines = Vec::new();
    for h in &harness::HARNESSES {
        let (steering, skill, agent) = paths(h, project)?;
        let mut line = format!("{:8} {}", h.name, project.display(&steering));
        for extra in [skill, agent].into_iter().flatten() {
            line.push_str("  ");
            line.push_str(&project.display(&extra));
        }
        lines.push(line);
    }
    Ok(Outcome {
        lines,
        exit: EXIT_OK,
        notices: Vec::new(),
    })
}

fn plan(opts: &Options) -> Result<Plan, AgentsError> {
    let name = opts
        .harness
        .as_deref()
        .ok_or_else(|| usage(AgentsPhrase::HarnessRequired.as_static()))?;
    let harness = harness::find(name).ok_or_else(|| {
        usage(AgentsText::HarnessUnknown {
            name: quoted(name),
            choices: harness::names().join(", "),
        })
    })?;
    if opts.check && opts.remove {
        return Err(usage(AgentsPhrase::CheckRemove.as_static()));
    }
    if opts.agent && harness.agent_format().is_none() {
        return Err(usage(AgentsText::NoAgentFile {
            harness: name.to_owned(),
        }));
    }
    if let Some(m) = &opts.reviewer_model {
        if !opts.agent {
            return Err(usage(AgentsPhrase::ModelNeedsAgent.as_static()));
        }
        if !content::valid_model(m) {
            return Err(usage(AgentsText::ModelInvalid { model: quoted(m) }));
        }
        if let Some(format) = harness.agent_format() {
            content::check_model_format(format, m).map_err(|message| {
                AgentsError::coded(&AgentsMessage::ReviewerModelFormat { message })
            })?;
        }
    }
    // `inherit` leaves the choice to the harness, which is the same as no model.
    let model = opts
        .reviewer_model
        .as_deref()
        .filter(|m| *m != content::INHERIT);
    let project = Project::open(&opts.root)?;
    let (steering, skill, agent) = paths(&harness, &project)?;
    Ok(Plan {
        project,
        steering,
        skill,
        agent,
        block_text: content::steering_block(opts.block),
        skill_text: (!opts.no_skill).then(content::skill_stub),
        agent_text: harness
            .agent_format()
            .filter(|_| opts.agent)
            .map(|fmt| content::reviewer_agent(fmt, model)),
        reviewer_model_unset: opts.agent && model.is_none(),
        harness,
    })
}

fn notices(p: &Plan) -> Result<Vec<Notice>, AgentsError> {
    if !p.reviewer_model_unset {
        return Ok(Vec::new());
    }
    let picks = match p.harness.agent_format() {
        Some(AgentFormat::Claude) => AgentsPhrase::PicksClaude,
        _ => AgentsPhrase::PicksOpencode,
    };
    Ok(vec![notice(
        NoticeLevel::Warning,
        &AgentsMessage::ReviewerModelUnset {
            picks: picks.as_static().to_owned(),
        },
    )?])
}

fn check(p: &Plan) -> Result<Outcome, AgentsError> {
    let mut states = vec![(
        block::block_state(&p.project, &p.steering, &p.block_text),
        &p.steering,
    )];
    if let Some(s) = &p.skill {
        states.push((managed::state(&p.project, s, p.skill_text.as_deref()), s));
    }
    if let Some(a) = &p.agent {
        states.push((managed::state(&p.project, a, p.agent_text.as_deref()), a));
    }
    let exit = if states.iter().any(|(s, _)| *s == State::Malformed) {
        EXIT_INCOMPLETE
    } else if states
        .iter()
        .all(|(s, _)| matches!(s, State::Current | State::Disabled))
    {
        EXIT_OK
    } else {
        EXIT_FINDINGS
    };
    let lines = states
        .iter()
        .map(|(s, path)| format!("{} {}", s.word(), p.project.display(path)))
        .collect();
    Ok(Outcome {
        lines,
        exit,
        notices: notices(p)?,
    })
}

fn skill_in_use_elsewhere(p: &Plan) -> bool {
    harness::HARNESSES
        .iter()
        .filter(|o| p.harness.shares_skills_with(o))
        .any(|o| {
            o.steering_path(&p.project).is_ok_and(|s| {
                s != p.steering
                    && block::block_state(&p.project, &s, &p.block_text) != State::Missing
            })
        })
}

fn remove(p: &Plan) -> Result<Outcome, AgentsError> {
    let project = &p.project;
    let mut lines = vec![status(
        project,
        Verb::Removed,
        block::remove_block(project, &p.steering)?,
        &p.steering,
    )];
    if let Some(s) = &p.skill {
        if skill_in_use_elsewhere(p) {
            lines.push(
                AgentsText::StatusKept {
                    path: project.display(s),
                }
                .to_string(),
            );
        } else {
            lines.push(status(
                project,
                Verb::Removed,
                managed::sync(project, s, None)?,
                s,
            ));
        }
    }
    if let Some(a) = &p.agent {
        lines.push(status(
            project,
            Verb::Removed,
            managed::sync(project, a, None)?,
            a,
        ));
    }
    Ok(Outcome {
        lines,
        exit: EXIT_OK,
        notices: Vec::new(),
    })
}

/// Whether `install` removes managed files that this run did not ask for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unrequested {
    /// `setup` semantics: no skill or reviewer text means delete the managed file.
    Remove,
    /// `init` semantics: leave such files alone.
    Keep,
}

fn install(p: &Plan, unrequested: Unrequested) -> Result<Outcome, AgentsError> {
    let project = &p.project;
    let skill = p
        .skill
        .as_ref()
        .filter(|_| p.skill_text.is_some() || unrequested == Unrequested::Remove);
    let agent = p
        .agent
        .as_ref()
        .filter(|_| p.agent_text.is_some() || unrequested == Unrequested::Remove);
    // Check every target before writing any, so a refusal leaves the project untouched.
    block::preflight(project, &p.steering, &p.block_text)?;
    for path in skill.into_iter().chain(agent) {
        managed::preflight(project, path)?;
    }
    let preamble = if p.harness.name == "kiro" && !project.exists(&p.steering)? {
        "---\ninclusion: always\n---"
    } else {
        ""
    };
    let mut lines = Vec::new();
    let mut changed_files = Vec::new();
    let mut step = |path: &Path, run: &dyn Fn() -> Result<bool, AgentsError>| match run() {
        Ok(changed) => {
            if changed {
                changed_files.push(path.display().to_string());
            }
            lines.push(status(project, Verb::Updated, changed, path));
            Ok(())
        }
        Err(source) if changed_files.is_empty() => Err(source),
        Err(source) => Err(AgentsError::Partial {
            changed: changed_files.clone(),
            source: Box::new(source),
        }),
    };
    step(&p.steering, &|| {
        block::update_block(project, &p.steering, &p.block_text, preamble)
    })?;
    if let Some(s) = skill {
        step(s, &|| managed::sync(project, s, p.skill_text.as_deref()))?;
    }
    if let Some(a) = agent {
        step(a, &|| managed::sync(project, a, p.agent_text.as_deref()))?;
    }
    Ok(Outcome {
        lines,
        exit: EXIT_OK,
        notices: notices(p)?,
    })
}

/// What a changed file reports: the managed text was taken out or put in.
#[derive(Clone, Copy)]
enum Verb {
    Removed,
    Updated,
}

fn status(project: &Project, verb: Verb, changed: bool, path: &Path) -> String {
    let path = project.display(path);
    match (changed, verb) {
        (false, _) => AgentsText::StatusUnchanged { path },
        (true, Verb::Removed) => AgentsText::StatusRemoved { path },
        (true, Verb::Updated) => AgentsText::StatusUpdated { path },
    }
    .to_string()
}

/// Run setup with explicit options.
pub fn execute(opts: &Options) -> Result<Outcome, AgentsError> {
    if opts.list {
        return list(&Project::open(&opts.root)?);
    }
    let p = plan(opts)?;
    if opts.check {
        check(&p)
    } else if opts.remove {
        remove(&p)
    } else {
        install(&p, Unrequested::Remove)
    }
}

/// Install steering without touching what the options do not ask for.
///
/// `--no-skill` leaves an existing managed skill in place, and an options set
/// without `agent` leaves an existing managed reviewer file in place. `execute`
/// deletes both. `slopvac init` uses this form so a re-run never removes files.
///
/// # Errors
/// As [`execute`]; also a usage error when `list`, `remove` or `check` is set.
pub fn install_keeping_unrequested(opts: &Options) -> Result<Outcome, AgentsError> {
    if opts.list || opts.remove || opts.check {
        return Err(usage(AgentsPhrase::InstallFlags.as_static()));
    }
    install(&plan(opts)?, Unrequested::Keep)
}
