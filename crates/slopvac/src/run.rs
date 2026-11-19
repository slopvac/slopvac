//! Parse, resolve global settings and dispatch.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgMatches;

use crate::cli;
use crate::color::{self, ColorChoice};
use crate::engine::{self, EngineLinter, LintRequest, Linter};
use crate::env::Env;
use crate::errors::CliError;
use crate::{
    exit, generate, init, inspect, lint_settings, logging, offline, output, owners, redact, stdin,
};

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
    // Not locked for the whole run: `slopvac lsp` writes from its own thread, which would
    // wait forever on a lock held here.
    main_with_stdout(args, env, &mut std::io::stdout())
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
    let cwd = std::env::current_dir().unwrap_or_default();
    let policy = match slopvac_update::policy::network_policy(&cwd) {
        Ok(policy) => policy,
        // `doctor` exists to report this state, so it runs on the default policy and its own
        // config scan shows the unreadable file (and exits 2 itself).
        Err(_) if invokes_doctor(&args, &tree) => slopvac_update::policy::NetworkPolicy::default(),
        Err(err) => {
            // policies.md 1.7: a path in an error is relative to the root or `<external>/<name>`.
            let message = format!(
                "{}: {}",
                redact::path_for_log(err.path(), &cwd),
                err.problem()
            );
            let _ = writeln!(
                err_out,
                "{}",
                CliError::new(err.code().unwrap_or("config-unreadable"), message)
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
        return match write_stdout(
            out,
            slopvac_update::command::version_json(&rt, &supported_versions()).as_bytes(),
        ) {
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

/// Whether these arguments run `slopvac doctor`. A parse failure or help request does not.
fn invokes_doctor(args: &[OsString], tree: &clap::Command) -> bool {
    let args = cli::insert_default_command(args.to_vec(), tree);
    tree.clone()
        .try_get_matches_from(args)
        .is_ok_and(|m| m.subcommand_name() == Some("doctor"))
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
        Some(("lint", m)) => lint(m, out, err_out),
        Some(("lsp", m)) => lsp(m).map(|exit| match exit {
            slopvac_lsp::Exit::AfterShutdown => exit::PASS,
            // LSP says `exit` without `shutdown` ends the process with status 1.
            slopvac_lsp::Exit::WithoutShutdown => exit::GATE_FAILED,
        }),
        Some(("migrate", m)) => migrate(m, out),
        Some(("completions", m)) => {
            let shell = *m
                .get_one::<clap_complete::Shell>("shell")
                .expect("required");
            write_stdout(out, &generate::completions(shell))
        }
        Some(("init", m)) => init::run(m, out),
        Some(("rules", m)) => {
            reject_removed(m, &cli::REMOVED_INSPECT_OPTIONS)?;
            inspect::rules(m, out)
        }
        Some(("explain", m)) => {
            reject_removed(m, &cli::REMOVED_INSPECT_OPTIONS)?;
            inspect::explain(m, out)
        }
        Some(("terms", m)) => owners::terms(m, out),
        Some(("genre", m)) => owners::genre(m, out),
        Some(("validate", m)) => owners::validate(m, out),
        Some(("setup", m)) => owners::setup(m, out),
        Some(("prime", m)) => owners::prime(m, out),
        Some(("review", m)) => owners::review(m, globals.offline, out),
        Some(("man", m)) => man(m, out),
        Some(("doctor", m)) => doctor(matches, m, env, out),
        Some(("cli-reference", _)) => write_stdout(
            out,
            generate::markdown_reference(&cli::command()).as_bytes(),
        ),
        Some((name, _)) => Err(CliError::new(
            "command-not-wired",
            format!("`slopvac {name}` is not available in this build yet."),
        )),
        None => {
            let mut cmd = cli::command();
            write_stdout(out, cmd.render_help().to_string().as_bytes())
        }
    }
}

/// `slopvac doctor`: print the report, and exit 2 when something needs attention.
fn doctor(
    root: &ArgMatches,
    m: &ArgMatches,
    env: &dyn Env,
    out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    use crate::doctor::stubs::{EnvDirs, LocalConfigSources};
    use crate::doctor::update::InstalledUpdateProbe;
    let cwd = std::env::current_dir().unwrap_or_default();
    let report = crate::doctor::build(&crate::doctor::Inputs {
        env,
        cwd: &cwd,
        offline_flag: root.get_flag("offline"),
        explicit_config: m.get_one::<PathBuf>("config").map(PathBuf::as_path),
        version: env!("CARGO_PKG_VERSION"),
        update: &InstalledUpdateProbe::new(),
        config: &LocalConfigSources,
        dirs: &EnvDirs,
        walk_limit: crate::doctor::WALK_LIMIT,
    });
    let text = if m.get_one::<String>("format").is_some_and(|f| f == "json") {
        report.to_json()
    } else {
        report.to_text()
    };
    write_stdout(out, text.as_bytes())?;
    Ok(if report.ok {
        exit::PASS
    } else {
        exit::INCOMPLETE
    })
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

fn migrate(m: &ArgMatches, out: &mut dyn std::io::Write) -> Result<u8, CliError> {
    let linter = slopvac_migrate::EngineLinter::new(vec![crate::rule_catalog::core()?])
        .map_err(|e| CliError::new(e.code(), e.to_string()))?;
    slopvac_migrate::execute(
        m,
        &linter,
        &slopvac_agents::content::steering_block(slopvac_agents::content::BlockKind::Standard),
        out,
    )
    .map(|()| exit::PASS)
    .map_err(|e| CliError::new(e.code(), e.to_string()))
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

/// Start the language server with the rules and the settings of the project config.
fn lsp(m: &ArgMatches) -> Result<slopvac_lsp::Exit, CliError> {
    let root = std::env::current_dir().unwrap_or_default();
    let linter = engine::project_linter(&root)?;
    slopvac_lsp::run(m, linter).map_err(|e| CliError::new("lsp-failed", e.to_string()))
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

fn lint(
    m: &ArgMatches,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    lint_with(m, &mut std::io::stdin().lock(), &EngineLinter, out, err_out)
}

fn lint_with(
    m: &ArgMatches,
    stdin_source: &mut dyn std::io::Read,
    linter: &dyn Linter,
    out: &mut dyn std::io::Write,
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
        config: m.get_one::<PathBuf>("config").cloned(),
        code_comments: m.get_one::<String>("mode").map(String::as_str) == Some("code-comments"),
        overrides: lint_settings::Overrides {
            min_score: m.get_one::<f64>("min-score").copied(),
            max_per_100_words: m.get_one::<f64>("max-per-100-words").copied(),
            only_categories: m
                .get_many::<String>("category")
                .map(|v| v.cloned().collect())
                .unwrap_or_default(),
            disable: m
                .get_many::<String>("disable")
                .map(|v| v.cloned().collect())
                .unwrap_or_default(),
        },
    });
    for err in &outcome.incomplete {
        let _ = writeln!(err_out, "{err}");
    }
    output::deliver(m, &outcome, out, err_out)?;
    tracing::info!(documents = outcome.documents, "lint done");
    Ok(if !outcome.incomplete.is_empty() {
        exit::INCOMPLETE
    } else if outcome.gate_failed {
        exit::GATE_FAILED
    } else {
        exit::PASS
    })
}

/// The versions this build supports, taken from the crates that own them. Keys are the file
/// names under `schemas/v3`, without the `.schema.json` suffix.
fn supported_versions() -> slopvac_update::command::Supported {
    // A config-style file is checked by `slopvac-config`, which owns its version; styles share it.
    let config = u64::try_from(slopvac_config::version::SUPPORTED).unwrap_or_default();
    let lock = u64::try_from(slopvac_plugins::lock::LOCK_SCHEMA_VERSION).unwrap_or_default();
    let report = u64::from(slopvac_model::SchemaVersion::VALUE);
    slopvac_update::command::Supported {
        schema_versions: std::collections::BTreeMap::from([
            (
                "blocklist",
                vec![u64::from(slopvac_terms::BLOCKLIST_SCHEMA_VERSION)],
            ),
            ("category", vec![report]),
            ("config", vec![config]),
            (
                "doctor-report",
                vec![u64::from(crate::doctor::SCHEMA_VERSION)],
            ),
            (
                "genre",
                vec![u64::from(slopvac_context::GENRE_SCHEMA_VERSION)],
            ),
            ("lint-report", vec![report]),
            ("packs-lock", vec![lock]),
            ("style", vec![config]),
            (
                "tagger",
                vec![u64::from(slopvac_context::TAGGER_SCHEMA_VERSION)],
            ),
            (
                "terms",
                vec![u64::from(slopvac_terms::TERMS_SCHEMA_VERSION)],
            ),
        ]),
        slopvac_api: slopvac_model::ApiVersion::SUPPORTED
            .iter()
            .map(|v| u64::from(*v))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RunOutcome;
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
        lint_with(m, &mut &*input, &recorder, &mut Vec::new(), &mut err).expect("lint runs");
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
