//! `slopvac init`: write a commented `slopvac.toml` and set up agent harnesses
//! (compat rows `cmd:init`, `opt:init:--force`, `--path`, `--skip-agents`, `--harness`,
//! `--no-skills`, `opt:init:--profile` (removed), `behavior:init-flags`,
//! `behavior:init-idempotence`).
//!
//! The template is `templates/slopvac.toml`. A test regenerates it from
//! `schemas/v3/config.schema.json` and fails when the two differ, so it cannot drift.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use slopvac_agents::{harness, setup};

use crate::errors::CliError;
use crate::{exit, redact};

#[cfg(test)]
mod template;

/// The checked-in starter config.
pub const TEMPLATE: &str = include_str!("../../templates/slopvac.toml");

/// The harness used when `--harness` is not given.
pub const DEFAULT_HARNESS: &str = "agents";

/// The `--harness` values.
#[must_use]
pub fn harness_names() -> Vec<&'static str> {
    harness::names()
}

/// How a path appears in a message: as given when relative, redacted when absolute.
fn shown(path: &Path) -> String {
    if path.is_absolute() {
        redact::path_for_log(path, &std::env::current_dir().unwrap_or_default())
    } else {
        path.display().to_string()
    }
}

/// Replace the project root in `text` so no absolute path leaves the process.
///
/// A root under the working directory becomes a relative path. Any other absolute root
/// becomes `<external>`, and the part below the root stays (policies.md, 1.7).
fn redact_root(text: &str, root: &Path) -> String {
    if !root.is_absolute() {
        return text.to_owned();
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let shown = match root.strip_prefix(&cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_owned(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => "<external>".to_owned(),
    };
    text.replace(&root.display().to_string(), &shown)
}

/// A temporary sibling that gets the mode a new file gets: `0666` less the umask.
///
/// The mode is applied when the file is created, so no umask is read or changed.
fn temp_sibling(parent: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    builder.tempfile_in(parent)
}

/// What publishing the config did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    /// The file was written.
    Wrote,
    /// A file was already there and `--force` was not set.
    Kept,
}

/// Write `text` to `path` through a temporary sibling, so a failed write leaves any
/// existing file as it was.
///
/// With `force` the sibling replaces the file in one step. Without it the sibling is
/// published only if no file exists by then; a file that appeared in between is kept.
///
/// # Errors
/// An I/O error from creating, writing or publishing the file.
pub fn publish(path: &Path, text: &str, force: bool) -> std::io::Result<Published> {
    publish_with(path, text, force, &|| {})
}

/// [`publish`] with a hook that runs after the sibling is written, for tests.
fn publish_with(
    path: &Path,
    text: &str,
    force: bool,
    before_publish: &dyn Fn(),
) -> std::io::Result<Published> {
    if !force && path.exists() {
        return Ok(Published::Kept);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = temp_sibling(parent)?;
    std::io::Write::write_all(&mut temp, text.as_bytes())?;
    if force && let Ok(existing) = std::fs::metadata(path) {
        // Replacing a file keeps its mode, as an in-place edit would.
        temp.as_file().set_permissions(existing.permissions())?;
    }
    temp.as_file().sync_all()?;
    before_publish();
    if force {
        temp.persist(path).map_err(|e| e.error)?;
        return Ok(Published::Wrote);
    }
    match temp.persist_noclobber(path) {
        Ok(_) => Ok(Published::Wrote),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(Published::Kept),
        Err(e) => Err(e.error),
    }
}

fn io_error(path: &Path, err: &std::io::Error) -> CliError {
    CliError::new(
        "output-failed",
        format!("{}: cannot write: {}", shown(path), err.kind()),
    )
}

/// Run `slopvac init`.
///
/// # Errors
/// A usage error for `--skip-agents` with harness options or for a directory
/// as `--path`; `output-failed` when the file cannot be written; the setup
/// error code when a harness fails.
pub fn run(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    if m.contains_id("profile")
        && let Some((_, message)) = crate::cli::REMOVED_LINT_OPTIONS
            .iter()
            .find(|(name, _)| *name == "profile")
    {
        return Err(CliError::new("removed-option", *message));
    }
    let path = m
        .get_one::<PathBuf>("path")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("slopvac.toml"));
    let harnesses: Vec<String> = m
        .get_many::<String>("harness")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let no_skills = m.get_flag("no-skills");
    let skip_agents = m.get_flag("skip-agents");
    if skip_agents && (!harnesses.is_empty() || no_skills) {
        return Err(CliError::new(
            "usage",
            "--skip-agents writes no steering; drop --harness and --no-skills.",
        ));
    }
    let say = |out: &mut dyn Write, line: String| {
        writeln!(out, "{line}").map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })
    };
    if path.is_dir() {
        return Err(CliError::new(
            "usage",
            format!("--path {} is a directory.", shown(&path)),
        ));
    }
    match publish(&path, TEMPLATE, m.get_flag("force")).map_err(|e| io_error(&path, &e))? {
        Published::Wrote => say(out, format!("wrote {}", shown(&path)))?,
        Published::Kept => say(out, format!("{} exists; keeping it.", shown(&path)))?,
    }
    if !skip_agents {
        let mut chosen: Vec<String> = Vec::new();
        for h in harnesses {
            if !chosen.contains(&h) {
                chosen.push(h);
            }
        }
        if chosen.is_empty() {
            chosen.push(DEFAULT_HARNESS.to_owned());
        }
        say(out, format!("harnesses: {}", chosen.join(", ")))?;
        let root = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        for name in chosen {
            let outcome = setup::install_keeping_unrequested(&setup::Options {
                harness: Some(name),
                list: false,
                remove: false,
                check: false,
                root: root.clone(),
                block: slopvac_agents::content::BlockKind::Standard,
                no_skill: no_skills,
                agent: false,
                reviewer_model: None,
            })
            .map_err(|err| CliError::new(err.code(), redact_root(&err.to_string(), &root)))?;
            for line in outcome.lines {
                say(out, redact_root(&line, &root))?;
            }
        }
    }
    say(out, "next: slopvac prime".to_owned())?;
    Ok(exit::PASS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_appears_before_publishing_is_kept_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slopvac.toml");
        let racer = file.clone();
        let result = publish_with(&file, "ours\n", false, &|| {
            std::fs::write(&racer, "theirs\n").unwrap();
        })
        .unwrap();
        assert_eq!(result, Published::Kept);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");
    }

    #[test]
    fn force_replaces_a_file_that_appeared_before_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slopvac.toml");
        let racer = file.clone();
        let result = publish_with(&file, "ours\n", true, &|| {
            std::fs::write(&racer, "theirs\n").unwrap();
        })
        .unwrap();
        assert_eq!(result, Published::Wrote);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "ours\n");
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slopvac.toml");
        publish(&file, "x\n", false).unwrap();
        publish(&file, "y\n", false).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["slopvac.toml"]);
    }

    #[test]
    fn an_external_root_is_redacted_but_keeps_the_part_below_it() {
        let text = "updated /srv/secret/proj/.claude/skills/x/SKILL.md";
        assert_eq!(
            redact_root(text, Path::new("/srv/secret/proj")),
            "updated <external>/.claude/skills/x/SKILL.md"
        );
    }

    #[test]
    fn a_relative_root_is_left_alone() {
        assert_eq!(
            redact_root("updated conf/AGENTS.md", Path::new("conf")),
            "updated conf/AGENTS.md"
        );
    }
}
