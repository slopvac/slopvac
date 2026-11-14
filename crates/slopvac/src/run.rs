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
use crate::{exit, generate, logging, offline, redact};

/// Settings every command shares, resolved once.
#[derive(Debug, Clone, Copy)]
struct Globals {
    offline: bool,
    color: bool,
}

fn globals(matches: &ArgMatches, env: &dyn Env) -> Globals {
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

/// Run the CLI with the given arguments and environment.
pub fn main_with<I, T>(args: I, env: &dyn Env) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let tree = cli::command();
    let args = cli::insert_default_command(args.into_iter().map(Into::into).collect(), &tree);
    let matches = match tree.try_get_matches_from(args) {
        Ok(m) => m,
        Err(err) => {
            let code = if err.use_stderr() {
                exit::INCOMPLETE
            } else {
                exit::PASS
            };
            let _ = err.print();
            return exit::code(code);
        }
    };
    let globals = globals(&matches, env);
    logging::init(env, globals.color);
    tracing::debug!(
        offline = globals.offline,
        color = globals.color,
        "resolved globals"
    );
    exit::code(dispatch(&matches, globals))
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    exit::INCOMPLETE
}

fn dispatch(matches: &ArgMatches, globals: Globals) -> u8 {
    match matches.subcommand() {
        Some(("lint", m)) => lint(m),
        Some(("completions", m)) => {
            let shell = *m
                .get_one::<clap_complete::Shell>("shell")
                .expect("required");
            generate::completions(shell, &mut std::io::stdout());
            exit::PASS
        }
        Some(("man", m)) => man(m),
        Some(("cli-reference", _)) => {
            print!("{}", generate::markdown_reference(&cli::command()));
            exit::PASS
        }
        Some((name, _)) => {
            let owner = cli::PLACEHOLDERS
                .iter()
                .find(|(n, ..)| *n == name)
                .map(|(_, _, o)| *o);
            let _ = globals;
            fail(&format!(
                "`slopvac {name}` is not available in this build yet ({}).",
                owner.unwrap_or("not wired")
            ))
        }
        None => {
            let _ = cli::command().print_help();
            exit::PASS
        }
    }
}

fn man(m: &ArgMatches) -> u8 {
    if let Some(dir) = m.get_one::<PathBuf>("out-dir") {
        return match generate::write_man_pages(dir) {
            Ok(()) => exit::PASS,
            Err(err) => fail(&format!("cannot write man pages: {}", err.kind())),
        };
    }
    match generate::man_page(&cli::command()) {
        Ok(page) => {
            let _ = std::io::stdout().write_all(&page);
            exit::PASS
        }
        Err(err) => fail(&format!("cannot render the man page: {}", err.kind())),
    }
}

fn lint(m: &ArgMatches) -> u8 {
    for (name, message) in REMOVED_LINT_OPTIONS {
        if m.contains_id(name) {
            return fail(message);
        }
    }
    let stdin = if m.get_flag("stdin") {
        let mut buf = Vec::new();
        if std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf).is_err() {
            return fail("cannot read standard input");
        }
        Some(buf)
    } else {
        None
    };
    let targets: Vec<PathBuf> = m
        .get_many::<PathBuf>("targets")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    if stdin.is_none() && targets.is_empty() {
        return fail("no input: pass paths or use --stdin");
    }
    tracing::info!(
        targets = targets.len(),
        stdin = stdin.is_some(),
        "lint start"
    );
    // STUB(slopvac-8l4.12.12): the real engine replaces StubLinter.
    let outcome = StubLinter.lint(&LintRequest { targets, stdin });
    for message in &outcome.incomplete {
        eprintln!("error: {}", redact::strip_url_userinfo(message));
    }
    tracing::info!(documents = outcome.documents, "lint done");
    if !outcome.incomplete.is_empty() {
        exit::INCOMPLETE
    } else if outcome.gate_failed {
        exit::GATE_FAILED
    } else {
        exit::PASS
    }
}
