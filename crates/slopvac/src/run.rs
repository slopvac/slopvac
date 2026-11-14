//! Parse, resolve global settings and dispatch.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgMatches;

use crate::cli::{self, REMOVED_LINT_OPTIONS};
use crate::color::{self, ColorChoice};
use crate::engine_stub::{LintRequest, Linter, StubLinter};
use crate::env::Env;
use crate::errors::CliError;
use crate::{exit, generate, logging, offline};

/// Settings every command shares, resolved once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Globals {
    /// Offline mode is on.
    pub offline: bool,
    /// Colour output is on.
    pub color: bool,
}

/// Resolve the shared settings from parsed arguments and the environment.
///
/// The flags are global, so they count before or after the subcommand.
#[must_use]
pub fn resolve_globals(matches: &ArgMatches, env: &dyn Env) -> Globals {
    let flag = if matches.get_flag("no-color") {
        Some(ColorChoice::Never)
    } else {
        matches
            .get_one::<String>("color")
            .and_then(|v| ColorChoice::parse(v))
    };
    Globals {
        offline: offline::resolve(matches.get_flag("offline"), env, None),
        color: color::resolve(flag, env),
    }
}

/// Read the colour flags from raw arguments, for output printed before parsing succeeds.
fn scan_color_flag(args: &[OsString]) -> Option<ColorChoice> {
    let mut choice = None;
    let mut iter = args.iter().skip(1).filter_map(|a| a.to_str());
    while let Some(arg) = iter.next() {
        if arg == "--no-color" {
            choice = Some(ColorChoice::Never);
        } else if arg == "--color" {
            choice = iter.next().and_then(ColorChoice::parse).or(choice);
        } else if let Some(value) = arg.strip_prefix("--color=") {
            choice = ColorChoice::parse(value).or(choice);
        }
    }
    choice
}

/// Run the CLI with the given arguments and environment.
pub fn main_with<I, T>(args: I, env: &dyn Env) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let tree = cli::command();
    let args = cli::insert_default_command(args.into_iter().map(Into::into).collect(), &tree);
    let early_color = color::resolve(scan_color_flag(&args), env);
    let tree = tree.color(if early_color {
        clap::ColorChoice::Always
    } else {
        clap::ColorChoice::Never
    });
    let matches = match tree.try_get_matches_from(args) {
        Ok(m) => m,
        Err(err) if err.use_stderr() => {
            let text = err.to_string();
            let body = text.strip_prefix("error: ").unwrap_or(&text);
            eprint!("error[usage]: {body}");
            return exit::code(exit::INCOMPLETE);
        }
        Err(err) => {
            let _ = err.print();
            return exit::code(exit::PASS);
        }
    };
    let globals = resolve_globals(&matches, env);
    logging::init(env, globals.color);
    tracing::debug!(
        offline = globals.offline,
        color = globals.color,
        "resolved globals"
    );
    match dispatch(&matches) {
        Ok(code) => exit::code(code),
        Err(err) => {
            eprintln!("{err}");
            exit::code(exit::INCOMPLETE)
        }
    }
}

fn dispatch(matches: &ArgMatches) -> Result<u8, CliError> {
    match matches.subcommand() {
        Some(("lint", m)) => lint(m),
        Some(("completions", m)) => {
            let shell = *m
                .get_one::<clap_complete::Shell>("shell")
                .expect("required");
            write_stdout(&generate::completions(shell))
        }
        Some(("man", m)) => man(m),
        Some(("cli-reference", _)) => {
            write_stdout(generate::markdown_reference(&cli::command()).as_bytes())
        }
        Some((name, _)) => {
            let owner = cli::PLACEHOLDERS
                .iter()
                .find(|(n, ..)| *n == name)
                .map(|(_, _, o)| *o)
                .or(match name {
                    "rules" | "explain" => Some("slopvac-8l4.12.11"),
                    "doctor" => Some("slopvac-8l4.12.5"),
                    "self" => Some("slopvac-8l4.12.4"),
                    "lsp" => Some("slopvac-8l4.12.7"),
                    _ => None,
                });
            Err(CliError::new(
                "command-not-wired",
                format!(
                    "`slopvac {name}` is not available in this build yet ({}).",
                    owner.unwrap_or("not wired")
                ),
            ))
        }
        None => {
            let mut cmd = cli::command();
            write_stdout(cmd.render_help().to_string().as_bytes())
        }
    }
}

fn write_stdout(bytes: &[u8]) -> Result<u8, CliError> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })?;
    Ok(exit::PASS)
}

fn man(m: &ArgMatches) -> Result<u8, CliError> {
    let failed = |err: std::io::Error| {
        CliError::new(
            "output-failed",
            format!("cannot write man pages: {}", err.kind()),
        )
    };
    if let Some(dir) = m.get_one::<PathBuf>("out-dir") {
        generate::write_man_pages(dir).map_err(failed)?;
        return Ok(exit::PASS);
    }
    let page = generate::man_page(&cli::command()).map_err(failed)?;
    write_stdout(&page)
}

fn lint(m: &ArgMatches) -> Result<u8, CliError> {
    for (name, message) in REMOVED_LINT_OPTIONS {
        if m.contains_id(name) {
            return Err(CliError::new("removed-option", message));
        }
    }
    let stdin = if m.get_flag("stdin") {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf).map_err(|_| {
            CliError::new("input-unreadable", "<stdin>: cannot read standard input")
        })?;
        Some(buf)
    } else {
        None
    };
    let targets: Vec<PathBuf> = m
        .get_many::<PathBuf>("targets")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    if stdin.is_none() && targets.is_empty() {
        return Err(CliError::new("no-input", "pass paths or use --stdin"));
    }
    tracing::info!(
        targets = targets.len(),
        stdin = stdin.is_some(),
        "lint start"
    );
    let root = std::env::current_dir().unwrap_or_default();
    // STUB(slopvac-8l4.12.12): the real engine replaces StubLinter.
    let outcome = StubLinter.lint(&LintRequest {
        root,
        targets,
        stdin,
    });
    for err in &outcome.incomplete {
        eprintln!("{err}");
    }
    tracing::info!(documents = outcome.documents, "lint done");
    Ok(if !outcome.incomplete.is_empty() {
        exit::INCOMPLETE
    } else if outcome.gate_failed {
        exit::GATE_FAILED
    } else {
        exit::PASS
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    fn globals_for(args: &[&str], vars: &[(&str, &str)], tty: bool) -> Globals {
        let tree = cli::command();
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let args = cli::insert_default_command(args, &tree);
        let matches = tree.try_get_matches_from(args).expect("parses");
        resolve_globals(&matches, &MapEnv::new(vars, tty))
    }

    #[test]
    fn flags_before_and_after_the_subcommand_both_count() {
        let cases: &[(&[&str], bool, bool)] = &[
            (&["slopvac", "--offline", "lint", "a.md"], true, false),
            (&["slopvac", "lint", "a.md", "--offline"], true, false),
            (&["slopvac", "a.md", "--offline"], true, false),
            (
                &["slopvac", "lint", "a.md", "--color", "always"],
                false,
                true,
            ),
            (&["slopvac", "--color=always", "doctor"], false, true),
            (&["slopvac", "lint", "a.md", "--no-color"], false, false),
            (&["slopvac", "lint", "a.md"], false, false),
        ];
        for (args, offline, color) in cases {
            let got = globals_for(args, &[], false);
            assert_eq!((got.offline, got.color), (*offline, *color), "{args:?}");
        }
    }

    #[test]
    fn environment_feeds_the_globals() {
        assert!(
            globals_for(
                &["slopvac", "lint", "a"],
                &[("SLOPVAC_OFFLINE", "1")],
                false
            )
            .offline
        );
        assert!(globals_for(&["slopvac", "lint", "a"], &[("FORCE_COLOR", "1")], false).color);
        assert!(!globals_for(&["slopvac", "lint", "a"], &[("NO_COLOR", "1")], true).color);
        assert!(
            !globals_for(
                &["slopvac", "lint", "a", "--color", "never"],
                &[("FORCE_COLOR", "1")],
                true
            )
            .color
        );
    }

    #[test]
    fn color_flag_scan_takes_the_last_value() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            scan_color_flag(&a(&["s", "--color", "always", "--no-color"])),
            Some(ColorChoice::Never)
        );
        assert_eq!(
            scan_color_flag(&a(&["s", "--color=always"])),
            Some(ColorChoice::Always)
        );
        assert_eq!(scan_color_flag(&a(&["s", "x"])), None);
    }
}
