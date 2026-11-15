//! Parse, resolve global settings and dispatch.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgMatches;

use crate::cli;
use crate::color::{self, ColorChoice};
use crate::engine_stub::{LintRequest, Linter, StubLinter};
use crate::env::Env;
use crate::errors::CliError;
use crate::{exit, generate, inspect, logging, offline, stdin};

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
    resolve_globals_with(matches, env, None)
}

/// Like [`resolve_globals`], with the project `[network] offline` value.
#[must_use]
pub fn resolve_globals_with(
    matches: &ArgMatches,
    env: &dyn Env,
    config_offline: Option<bool>,
) -> Globals {
    let flag = if matches.get_flag("no-color") {
        Some(ColorChoice::Never)
    } else {
        matches
            .get_one::<String>("color")
            .and_then(|v| ColorChoice::parse(v))
    };
    Globals {
        offline: offline::resolve(matches.get_flag("offline"), env, config_offline),
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

/// Run the CLI with the given arguments and environment, writing to the real stdout.
pub fn main_with<I, T>(args: I, env: &dyn Env) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    main_with_stdout(args, env, &mut std::io::stdout().lock())
}

/// Like [`main_with`], with the standard output stream supplied by the caller.
pub fn main_with_stdout<I, T>(args: I, env: &dyn Env, out: &mut dyn std::io::Write) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    main_with_io(args, env, out, &mut std::io::stderr().lock())
}

/// Like [`main_with`], with both output streams supplied by the caller.
pub fn main_with_io<I, T>(
    args: I,
    env: &dyn Env,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let tree = cli::command();
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let policy = match slopvac_update::policy::network_policy(
        &std::env::current_dir().unwrap_or_default(),
    ) {
        Ok(policy) => policy,
        Err(err) => {
            let _ = writeln!(
                err_out,
                "{}",
                CliError::new("config-unreadable", err.to_string())
            );
            return exit::code(exit::INCOMPLETE);
        }
    };
    let config_offline = policy.offline.then_some(true);
    // `--version --json` is read before the default `lint` command is inserted.
    if let Some(flag) = version_json_offline_flag(&args) {
        let rt = update_runtime(
            env,
            offline::resolve(flag, env, config_offline),
            policy.update_check,
        );
        return match write_stdout(out, slopvac_update::command::version_json(&rt).as_bytes()) {
            Ok(code) => exit::code(code),
            Err(failure) => {
                let _ = writeln!(err_out, "{failure}");
                exit::code(exit::INCOMPLETE)
            }
        };
    }
    let args = cli::insert_default_command(args, &tree);
    let early_color = color::resolve(scan_color_flag(&args), env);
    let tree = tree.color(if early_color {
        clap::ColorChoice::Always
    } else {
        clap::ColorChoice::Never
    });
    let matches = match tree.try_get_matches_from(args) {
        Ok(m) => m,
        Err(err) if err.use_stderr() => {
            let styled = err.render();
            let text = if early_color {
                styled.ansi().to_string()
            } else {
                styled.to_string()
            };
            let _ = write!(err_out, "{}", text.replacen("error:", "error[usage]:", 1));
            return exit::code(exit::INCOMPLETE);
        }
        Err(err) => {
            let styled = err.render();
            let text = if early_color {
                styled.ansi().to_string()
            } else {
                styled.to_string()
            };
            return match write_stdout(out, text.as_bytes()) {
                Ok(code) => exit::code(code),
                Err(failure) => {
                    let _ = writeln!(err_out, "{failure}");
                    exit::code(exit::INCOMPLETE)
                }
            };
        }
    };
    let globals = resolve_globals_with(&matches, env, config_offline);
    logging::init(env, globals.color);
    tracing::debug!(
        offline = globals.offline,
        color = globals.color,
        "resolved globals"
    );
    match dispatch(&matches, globals, policy.update_check, env, out, err_out) {
        Ok(code) => exit::code(code),
        Err(err) => {
            let _ = writeln!(err_out, "{err}");
            exit::code(exit::INCOMPLETE)
        }
    }
}

fn dispatch(
    matches: &ArgMatches,
    globals: Globals,
    update_check: Option<bool>,
    env: &dyn Env,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    match matches.subcommand() {
        Some(("self", m)) => {
            let rt = update_runtime(env, globals.offline, update_check);
            let text = slopvac_update::command::run(m, &rt)
                .map_err(|f| CliError::new(f.code, f.message))?;
            write_stdout(out, text.as_bytes())
        }
        Some(("lint", m)) => lint(m, err_out),
        Some(("completions", m)) => {
            let shell = *m
                .get_one::<clap_complete::Shell>("shell")
                .expect("required");
            write_stdout(out, &generate::completions(shell))
        }
        Some(("rules", m)) => {
            reject_removed(m, &cli::REMOVED_INSPECT_OPTIONS)?;
            inspect::rules(m, out)
        }
        Some(("explain", m)) => {
            reject_removed(m, &cli::REMOVED_INSPECT_OPTIONS)?;
            inspect::explain(m, out)
        }
        Some(("man", m)) => man(m, out),
        Some(("cli-reference", _)) => write_stdout(
            out,
            generate::markdown_reference(&cli::command()).as_bytes(),
        ),
        Some((name, _)) => {
            let owner = cli::PLACEHOLDERS
                .iter()
                .find(|(n, ..)| *n == name)
                .map(|(_, _, o)| *o)
                .or(match name {
                    "rules" | "explain" => Some("slopvac-8l4.12.11"),
                    "doctor" => Some("slopvac-8l4.12.5"),
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
            write_stdout(out, cmd.render_help().to_string().as_bytes())
        }
    }
}

/// Fail with the 3.0 pointer when a removed 2.x option was given.
fn reject_removed(m: &ArgMatches, names: &[&str]) -> Result<(), CliError> {
    for name in names {
        if m.contains_id(name)
            && let Some(message) = cli::removed_message(name)
        {
            return Err(CliError::new("removed-option", message));
        }
    }
    Ok(())
}

/// For `--version --json` with only global flags around it, whether `--offline` was among them.
///
/// Returns `None` for any other invocation. The flags may come in any order.
fn version_json_offline_flag(args: &[OsString]) -> Option<bool> {
    let (mut version, mut json, mut offline) = (false, false, false);
    let mut tokens = args.iter().skip(1).map(|a| a.to_str());
    while let Some(token) = tokens.next() {
        match token? {
            "--version" | "-V" => version = true,
            "--json" => json = true,
            "--offline" => offline = true,
            "--no-color" => {}
            "--color" => {
                ColorChoice::parse(tokens.next()??)?;
            }
            t if t
                .strip_prefix("--color=")
                .is_some_and(|v| ColorChoice::parse(v).is_some()) => {}
            _ => return None,
        }
    }
    (version && json).then_some(offline)
}

/// Build the update runtime from the injected environment.
fn update_runtime(
    env: &dyn Env,
    offline: bool,
    update_check: Option<bool>,
) -> slopvac_update::command::Runtime {
    let vars: std::collections::HashMap<&'static str, String> = slopvac_update::command::ENV_VARS
        .iter()
        .filter_map(|name| env.var(name).map(|v| (*name, v)))
        .collect();
    slopvac_update::command::Runtime::with_var(
        offline,
        env!("CARGO_PKG_VERSION"),
        update_check,
        Box::new(move |k| vars.get(k).cloned()),
    )
}

fn write_stdout(out: &mut dyn std::io::Write, bytes: &[u8]) -> Result<u8, CliError> {
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

fn man(m: &ArgMatches, out: &mut dyn std::io::Write) -> Result<u8, CliError> {
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
    write_stdout(out, &page)
}

fn lint(m: &ArgMatches, err_out: &mut dyn std::io::Write) -> Result<u8, CliError> {
    // STUB(slopvac-8l4.12.12): the real engine replaces StubLinter.
    lint_with(m, &mut std::io::stdin().lock(), &StubLinter, err_out)
}

fn lint_with(
    m: &ArgMatches,
    stdin_source: &mut dyn std::io::Read,
    linter: &dyn Linter,
    err_out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    reject_removed(m, &cli::REMOVED_LINT_NAMES)?;
    let stdin = if m.get_flag("stdin") {
        let plan = stdin::plan(m)?;
        tracing::debug!(language = plan.language, mode = plan.mode, "stdin plan");
        Some((plan, stdin::read(stdin_source)?))
    } else {
        None
    };
    let (plan, stdin) = match stdin {
        Some((plan, bytes)) => (Some(plan), Some(bytes)),
        None => (None, None),
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
    let outcome = linter.lint(&LintRequest {
        root,
        targets,
        stdin,
        stdin_plan: plan,
    });
    for err in &outcome.incomplete {
        let _ = writeln!(err_out, "{err}");
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
    use crate::engine_stub::RunOutcome;
    use crate::env::MapEnv;
    use crate::stdin::StdinPlan;
    use std::cell::RefCell;

    /// Records what the CLI hands to the engine.
    #[derive(Default)]
    struct Recorder(RefCell<Vec<LintRequest>>);

    impl Linter for Recorder {
        fn lint(&self, request: &LintRequest) -> RunOutcome {
            self.0.borrow_mut().push(request.clone());
            RunOutcome::default()
        }
    }

    fn requests_for(args: &[&str], input: &[u8]) -> Vec<LintRequest> {
        let tree = cli::command();
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let matches = tree.try_get_matches_from(args).expect("parses");
        let (_, m) = matches.subcommand().expect("lint");
        let recorder = Recorder::default();
        let mut err = Vec::new();
        lint_with(m, &mut &*input, &recorder, &mut err).expect("lint runs");
        recorder.0.into_inner()
    }

    #[test]
    fn stdin_language_and_mode_reach_the_linter() {
        let cases: &[(&[&str], &str, &str)] = &[
            (&["slopvac", "lint", "--stdin"], "markdown", "prose"),
            (
                &["slopvac", "lint", "--stdin", "--language", "html"],
                "html",
                "prose",
            ),
            (
                &[
                    "slopvac",
                    "lint",
                    "--stdin",
                    "--mode",
                    "code-comments",
                    "--language",
                    "python",
                ],
                "python",
                "code-comments",
            ),
        ];
        for (args, language, mode) in cases {
            let got = requests_for(args, b"x\n");
            assert_eq!(got.len(), 1, "{args:?}");
            assert_eq!(
                got[0].stdin_plan,
                Some(StdinPlan {
                    mode,
                    language: (*language).to_owned()
                }),
                "{args:?}"
            );
            assert_eq!(got[0].stdin.as_deref(), Some(&b"x\n"[..]), "{args:?}");
        }
    }

    #[test]
    fn file_runs_carry_no_stdin_plan() {
        let got = requests_for(&["slopvac", "lint", "a.md"], b"");
        assert_eq!(got[0].stdin_plan, None);
    }

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
