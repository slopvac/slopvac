//! Commands whose logic lives in an owner crate: `terms`, `genre`, `validate`, `setup`,
//! `prime`, `review` and `claims`. Each owner exposes `command()` and a run function; this module maps their
//! results and errors onto the CLI's exit codes and error codes (policies.md, 3.5) and
//! keeps absolute paths out of every message (policies.md, 1.7).

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use slopvac_agents::claims;
use slopvac_agents::review::{self, ReviewEnv};
use slopvac_agents::{AgentsError, prime, setup};
use slopvac_config::Discovery;
use slopvac_styles::{BuiltinSource, StylePresets};
use slopvac_terms::TermsError;

use crate::errors::CliError;
use crate::{cli, engine, exit, redact};

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

fn write(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })
}

/// A byte code from an owner crate's exit status.
fn exit_code(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(exit::INCOMPLETE)
}

fn shown(path: &str) -> String {
    redact::shown_path(Path::new(path), &cwd())
}

fn terms_error(err: &TermsError) -> CliError {
    let (code, path) = match err {
        TermsError::Io { path, .. } => ("terms-io", path),
        TermsError::TooLarge { code, path, .. } => (*code, path),
        TermsError::Parse { path, .. } => ("terms-parse", path),
        TermsError::MissingSchemaVersion { path } => ("terms-missing-schema-version", path),
        TermsError::SchemaVersion { path, .. } => ("terms-schema-version", path),
        TermsError::UnknownKey { path, .. } => ("terms-unknown-key", path),
        TermsError::Entry { path, .. } => ("terms-entry", path),
    };
    // The owner's own wording, with its path (carried as data) replaced whole.
    let message = err.to_string().replace(path.as_str(), &shown(path));
    CliError::new(code, redact::redact_root(&message, &cwd()))
}

/// Name an owner's error from its fields. Every path the owner reports is relative to the
/// project `root`; it is joined to `root` and shown through `shown_path`, so it reads relative
/// to the run root or as `<external>/<file name>` outside it (policies.md, 1.7).
pub(crate) fn agents_error(err: &AgentsError, root: &Path) -> CliError {
    let error = CliError::new(err.code(), agents_message(err, root));
    match err.budget() {
        Some((budget, limit)) => {
            let mut data = crate::engine::budget_data_of(budget, limit);
            if let Some(path) = err.input_path() {
                data.insert("path".into(), path.into());
            }
            error.with_data(data)
        }
        None => error,
    }
}

fn agents_message(err: &AgentsError, root: &Path) -> String {
    let cwd = cwd();
    // Every path in an `AgentsError` is relative to the root the caller passed. An absolute
    // path (a file the user named on the command line) replaces the root in `join`.
    let path = |p: &Path| redact::shown_path(&root.join(p), &cwd);
    match err {
        AgentsError::UnsafeTarget(target) => {
            format!("unsafe steering target: {}", path(Path::new(target)))
        }
        AgentsError::MalformedMarkers(p) => format!(
            "malformed or ambiguous slopvac markers in {}; repair the begin/end pair first",
            path(p)
        ),
        AgentsError::UnclosedFence(p) => format!(
            "{} has an unterminated code fence that would hide the slopvac block; close the fence first",
            path(p)
        ),
        AgentsError::Partial { changed, source } => format!(
            "{}; already changed: {}",
            agents_message(source, root),
            changed
                .iter()
                .map(|c| path(Path::new(c)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AgentsError::Text(e) => format!("text segmentation failed: {e}"),
        AgentsError::Context(e) => e.to_string(),
        AgentsError::InputBudget { shown, source } => {
            format!("{}: {source}", slopvac_sanitize::line(shown))
        }
        AgentsError::NotUtf8(p) => format!("steering must be UTF-8: {}", path(p)),
        AgentsError::Conflict(p) => format!("file changed during setup; retry: {}", path(p)),
        AgentsError::NotManaged(p) => format!(
            "{} is not a slopvac-managed file; move it aside to install it",
            path(p)
        ),
        AgentsError::Io { path: p, source } => format!("{}: {source}", path(p)),
        AgentsError::Usage(m) | AgentsError::Coded { message: m, .. } => m.clone(),
        // The config crate's rendered text, which leads with its own `error[code]:` and names
        // paths as the user typed them.
        AgentsError::Config { code, source } => {
            let rendered = source.diagnostics();
            let body = rendered
                .strip_prefix(&format!("error[{code}]: "))
                .unwrap_or(&rendered);
            // Replace each path the owner named, whole, before any text inference.
            let mut body = body.to_owned();
            for named in source.paths() {
                body = body.replace(named.as_str(), &shown(&named));
            }
            redact::redact_absolute(&body, &cwd)
        }
    }
}

/// `slopvac terms`.
pub fn terms(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    slopvac_terms::cli::run(m, &cwd(), out)
        .map(exit_code)
        .map_err(|e| terms_error(&e))
}

/// `slopvac genre`.
pub fn genre(m: &ArgMatches, out: &mut dyn Write, err_out: &mut dyn Write) -> Result<u8, CliError> {
    let root = cwd();
    let presets = StylePresets(BuiltinSource);
    let discovery = Discovery::new(&presets);
    // The front matter policy is the project's: the nearest config of the directory of the
    // resolved file, as `lint` finds it. A config that cannot be read stops the command.
    let config_failure = std::cell::RefCell::new(None);
    let block_failures = std::cell::RefCell::new(Vec::new());
    let mut declared = |path: &str, text: &str| {
        if config_failure.borrow().is_some() {
            return None;
        }
        let absolute = root.join(path);
        let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        let dir = resolved.parent().unwrap_or(&root);
        match discovery.nearest(dir) {
            Ok(config) => {
                let shown = shown(path);
                let read = engine::read_block_under(text, &shown, config.as_deref());
                block_failures
                    .borrow_mut()
                    .extend(engine::block_errors(&shown, text, &read));
                engine::declared_genre(&read)
            }
            Err(err) => {
                *config_failure.borrow_mut() = Some(engine::config_error(&err, &root));
                None
            }
        }
    };
    let result = slopvac_context::run(m, &mut declared);
    if let Some(failure) = config_failure.into_inner() {
        return Err(failure);
    }
    let text = result.map_err(|e| {
        // The owner words a read failure as `<path as typed>: <reason>`. Replace each file
        // argument, exactly as typed, with its shown form.
        let mut message = e.to_string();
        for file in m.get_many::<String>("files").into_iter().flatten() {
            message = message.replace(file.as_str(), &shown(file));
        }
        match e {
            // The same code and wording as the lint path's refusal of an oversized document.
            slopvac_context::Error::TooLarge { path, .. } => CliError::new(
                "document-too-large",
                format!(
                    "{}: larger than the 64 MiB document limit",
                    slopvac_sanitize::line(&shown(&path))
                ),
            ),
            // The lint path's code and wording for a file it refuses to read.
            slopvac_context::Error::Unreadable { path } => CliError::new(
                "input-unreadable",
                format!("{}: cannot read", slopvac_sanitize::line(&shown(&path))),
            ),
            _ => CliError::new("genre-failed", message),
        }
    })?;
    write(out, &text)?;
    // A block that fails validation is an error every time, as in `lint`.
    let failures = block_failures.into_inner();
    for failure in &failures {
        let _ = writeln!(err_out, "{}", CliError::new(failure.code, &failure.escaped));
    }
    Ok(if failures.is_empty() {
        exit::PASS
    } else {
        exit::INCOMPLETE
    })
}

/// `slopvac validate`.
pub fn validate(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let presets = StylePresets(BuiltinSource);
    slopvac_config::validate::run(m, &cwd(), &presets, out).map_err(|err| {
        CliError::new(
            "output-failed",
            format!("cannot write output: {}", err.kind()),
        )
    })
}

/// `slopvac setup`.
pub fn setup(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let root = m
        .get_one::<PathBuf>("root")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    let outcome = setup::run(m).map_err(|e| agents_error(&e, &root))?;
    for line in &outcome.lines {
        write(out, &format!("{}\n", redact::redact_root(line, &root)))?;
    }
    Ok(outcome.exit)
}

/// `slopvac prime`.
pub fn prime(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let text = prime::run(m, &cli::command(), &cwd()).map_err(|e| agents_error(&e, &cwd()))?;
    write(out, &text)?;
    Ok(exit::PASS)
}

/// `slopvac review`.
///
/// `[review] provider`, `model` and `author_model` come from the resolved project config, and
/// the flags win over them. The provider plugin starts only when the run calls it. A run that
/// stops prints no plan; the notices known before it stopped go to `err_out`.
pub fn review(
    m: &ArgMatches,
    offline: bool,
    env: &dyn crate::env::Env,
    out: &mut dyn Write,
    err_out: &mut dyn Write,
) -> Result<u8, CliError> {
    let root = cwd();
    let presets = StylePresets(BuiltinSource);
    let config = Discovery::new(&presets)
        .nearest(&root)
        .map_err(|e| engine::config_error(&e, &root))?;
    let value = |key: &str| {
        config
            .as_ref()
            .and_then(|c| c.project_value(&["review", key]))
            .and_then(|v| v.as_str().map(str::to_owned))
    };
    let provider = value("provider").map(|id| {
        crate::review_provider::PluginProvider::new(id, offline, env, &root, config.as_deref())
    });
    let review_env = ReviewEnv {
        offline: crate::offline::project(offline, env, config.as_deref()),
        provider: provider.as_ref().map(|p| p as &dyn review::ModelProvider),
        author_model: value("author_model"),
        reviewer_model: value("model"),
    };
    match review::run(m, &review_env) {
        Ok(text) => {
            write(out, &text)?;
            Ok(exit::PASS)
        }
        Err(stopped) => Err(stopped_error(&stopped, err_out, &root)),
    }
}

/// The error of a review that stopped, after its notices went to `err_out`. A notice that
/// cannot be written must not hide the error: the error is what the run reports. The first
/// failed write ends the notices, because the stream is broken.
fn stopped_error(stopped: &review::Stopped, err_out: &mut dyn Write, root: &Path) -> CliError {
    for notice in &stopped.notices {
        if writeln!(err_out, "{}", slopvac_report::render_notice(notice)).is_err() {
            // The stream is broken, so the notices after this one cannot reach it either.
            break;
        }
    }
    agents_error(&stopped.error, root)
}

/// `slopvac claims`. Reads the two named files and nothing else, so it needs no network.
pub fn claims(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let text = match claims::run(m) {
        Ok(text) => text,
        Err(e) => {
            let error = agents_error(&e, &cwd());
            // The JSON report is the default format. A refused budget reports as a lint run
            // does, so a program reads the budget and its limit from the notice data. The
            // text error still goes to stderr, and the exit code is the same.
            let json = m.get_one::<String>("format").is_none_or(|f| f != "text");
            if json && e.budget().is_some() {
                let report = slopvac_report::render(
                    slopvac_report::Format::Json,
                    &engine::failure_report(&error),
                    &slopvac_report::NoSources,
                )
                .map_err(|err| CliError::new("render-failed", err.to_string()))?;
                write(out, &report)?;
            }
            return Err(error);
        }
    };
    write(out, &text)?;
    Ok(exit::PASS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// A writer that refuses every write.
    struct Refusing;

    impl Write for Refusing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    /// A writer that counts its calls and refuses each one.
    #[derive(Default)]
    struct Counting(usize);

    impl Write for Counting {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.0 += 1;
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn warning(code: &str) -> slopvac_model::Notice {
        slopvac_model::Notice::from_catalog(
            slopvac_model::NoticeLevel::Warning,
            slopvac_model::NoticeMessage::new(
                slopvac_model::KebabName::new(code).unwrap(),
                slopvac_model::NonEmptyString::new("a notice").unwrap(),
            ),
        )
    }

    /// The first notice that cannot be written ends the notices: the stream is broken, and the
    /// review error is still the result.
    #[test]
    fn a_failed_notice_write_ends_the_notices_and_keeps_the_error() {
        let stopped = review::Stopped::new(
            AgentsError::new_coded("review-provider-missing", "no provider"),
            vec![
                warning("review-model-unchecked"),
                warning("review-model-same"),
            ],
        );
        let mut err_out = Counting::default();

        let error = stopped_error(&stopped, &mut err_out, Path::new("proj"));

        assert_eq!(error.code, "review-provider-missing");
        assert_eq!(err_out.0, 1, "writes after the first failed one");
    }

    /// A stopped review whose notices cannot be written still reports its own error.
    #[test]
    fn a_notice_that_cannot_be_written_leaves_the_review_error() {
        let notice = slopvac_model::Notice::from_catalog(
            slopvac_model::NoticeLevel::Warning,
            slopvac_model::NoticeMessage::new(
                slopvac_model::KebabName::new("review-model-unchecked").unwrap(),
                slopvac_model::NonEmptyString::new("a notice").unwrap(),
            ),
        );
        let stopped = review::Stopped::new(
            AgentsError::new_coded("review-provider-missing", "no provider"),
            vec![notice],
        );

        let error = stopped_error(&stopped, &mut Refusing, Path::new("proj"));

        assert_eq!(error.code, "review-provider-missing");
    }

    // Owners report paths relative to the project. With a relative root, each variant shows
    // the root once.
    #[rstest]
    #[case::io(AgentsError::Io {
        path: PathBuf::from("AGENTS.md"),
        source: std::io::Error::from(std::io::ErrorKind::NotFound),
    }, "setup-io", "proj/AGENTS.md: ")]
    #[case::unsafe_target(
        AgentsError::UnsafeTarget("AGENTS.md".to_owned()),
        "steering-unsafe-target",
        "unsafe steering target: proj/AGENTS.md"
    )]
    #[case::conflict(
        AgentsError::Conflict(PathBuf::from("AGENTS.md")),
        "steering-conflict",
        "retry: proj/AGENTS.md"
    )]
    #[case::relative_to_the_project(
        AgentsError::NotUtf8(PathBuf::from("AGENTS.md")),
        "steering-not-utf8",
        "UTF-8: proj/AGENTS.md"
    )]
    fn a_relative_root_is_shown_once(
        #[case] err: AgentsError,
        #[case] code: &str,
        #[case] expected: &str,
    ) {
        let shown = agents_error(&err, Path::new("proj"));
        let text = shown.to_string();
        assert!(text.starts_with(&format!("error[{code}]: ")), "{text}");
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("proj/proj"), "{text}");
    }
}
