//! The CLI-facing failure, budgeted discovery, capped diagnostics, excluded candidates,
//! the schema version type and MDX handling.

use std::fs;
use std::path::Path;

use slopvac_migrate::{
    CliFailure, Error, Finding, Limits, Linter, NullLinter, Options, Phase, RenameTable, command,
    execute, migrate_config, plan, plan_with_limits, run,
};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";
const DIRECTIVE: &str =
    "<!-- slopvac-allow: rule=prose-format.no-unicode-dash reason=x -->\nText\n";
const UNKNOWN: &str = "<!-- slopvac-allow: rule=no.such-rule reason=x -->\n";
const INVALID: &str = "<!-- slopvac-allow: rule=a.b,c.d reason=x -->\n\n";
const REMOVED: &str =
    "<!-- slopvac-allow: rule=ste-words.inconsistent-term-for-same-thing reason=x -->\n\n";

fn write(root: &Path, name: &str, text: impl AsRef<[u8]>) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn plan_of(root: &Path) -> Result<slopvac_migrate::Plan, slopvac_migrate::Error> {
    plan(root, &RenameTable::bundled(), Options::default(), STEERING)
}

fn cli(root: &Path) -> (i32, String, String) {
    let m = command().get_matches_from(["migrate", root.to_str().unwrap()]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let state = tempfile::tempdir().unwrap();
    let code = run(&m, &NullLinter, STEERING, state.path(), &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn exec(root: &Path) -> Result<String, CliFailure> {
    let m = command().get_matches_from(["migrate", root.to_str().unwrap()]);
    let mut out = Vec::new();
    let state = tempfile::tempdir().unwrap();
    execute(&m, &NullLinter, STEERING, state.path(), &mut out)
        .map(|()| String::from_utf8(out).unwrap())
}

#[test]
fn the_failure_execute_returns_is_escaped_once_and_keeps_the_raw_source() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "docs/a\\b\u{1b}.md", UNKNOWN);
    let f = exec(dir.path()).unwrap_err();
    assert_eq!(f.code(), "config-legacy-dropped");
    let shown = f.to_string();
    assert!(!shown.contains('\u{1b}'), "{shown:?}");
    // One backslash in the name becomes two, not four.
    assert!(shown.contains("docs/a\\\\b\\u{1b}.md"), "{shown:?}");
    assert!(!shown.contains("a\\\\\\\\b"), "{shown:?}");
    let raw = std::error::Error::source(&f).unwrap().to_string();
    assert!(raw.contains("a\\b\u{1b}.md"), "{raw:?}");
}

#[test]
fn discovered_entries_count_against_the_file_limit_before_any_read() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        write(dir.path(), &format!("src/f{i}.rs"), "");
    }
    let e = plan_with_limits(
        dir.path(),
        &RenameTable::bundled(),
        Options::default(),
        STEERING,
        Limits::new(3, u64::MAX),
    )
    .unwrap_err();
    assert_eq!(e.code(), "migrate-run-limit");
    assert!(e.to_string().contains("/src/f"), "{e}");
}

#[test]
fn warnings_are_capped_with_one_suppressed_line() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", INVALID.repeat(250));
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.warnings.len(), 101);
    assert_eq!(p.warnings[100], "150 more warnings suppressed");
}

#[test]
fn notes_are_capped_with_one_suppressed_line() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", REMOVED.repeat(250));
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.notes.len(), 101);
    assert_eq!(p.notes[100], "150 more notes suppressed");
}

#[test]
fn one_message_is_cut_to_its_length_cap() {
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(200);
    write(
        dir.path(),
        &format!("{long}/{long}/{long}/a.md"),
        "<!-- slopvac-allow: rule=a.b,c.d reason=x -->\n",
    );
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].chars().count(), 513, "{:?}", p.warnings[0]);
    assert!(p.warnings[0].ends_with('…'));
}

#[cfg(unix)]
mod excluded {
    use super::*;
    use std::os::unix::fs::symlink;

    fn warns(root: &Path, prefix: &str) -> bool {
        let w = plan_of(root).unwrap().warnings;
        w.iter().any(|m| m.starts_with(prefix)) || {
            eprintln!("{w:?}");
            false
        }
    }

    #[test]
    fn a_symlinked_document_is_reported_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "real.txt", "x");
        symlink("real.txt", dir.path().join("link.md")).unwrap();
        assert!(warns(dir.path(), "link.md: not checked: symbolic link"));
    }

    #[test]
    fn a_dangling_symlink_candidate_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        symlink("missing", dir.path().join("slopvac.toml")).unwrap();
        assert!(warns(
            dir.path(),
            "slopvac.toml: not checked: symbolic link"
        ));
    }

    #[test]
    fn a_symlink_cycle_is_reported_and_the_walk_ends() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sub/a.txt", "x");
        symlink("..", dir.path().join("sub/up")).unwrap();
        assert!(warns(dir.path(), "sub/up: not checked: symbolic link"));
    }

    #[test]
    fn a_special_file_with_a_document_name_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join("pipe.md"))
            .status()
            .unwrap()
            .success();
        assert!(made);
        assert!(warns(
            dir.path(),
            "pipe.md: not checked: not a regular file"
        ));
    }
}

#[test]
fn a_non_integer_schema_version_is_exit_2_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "slopvac.toml",
        "profile = \"strict\"\nschema_version = \"3\"\n",
    );
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[config-invalid-type]: slopvac.toml:2: "),
        "{err}"
    );
}

#[test]
fn a_non_integer_schema_version_in_pyproject_names_the_pyproject_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.slopvac]\nschema_version = 3.5\n",
    );
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[config-invalid-type]: pyproject.toml:5: "),
        "{err}"
    );
}

#[test]
fn an_exported_string_in_mdx_is_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.mdx",
        "export const d = \"<!-- slopvac-allow: rule=prose-format.no-unicode-dash reason=x -->\";\n",
    );
    assert_eq!(plan_of(dir.path()).unwrap().changes.len(), 0);
}

#[test]
fn an_mdx_directive_that_needs_a_rename_is_a_warning_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mdx", format!("# T\n\n{DIRECTIVE}"));
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.changes.len(), 0);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(
        p.warnings[0].starts_with("a.mdx:3: MDX is not rewritten"),
        "{:?}",
        p.warnings
    );
}

struct Hostile;

impl Linter for Hostile {
    fn lint(
        &self,
        _root: &Path,
        phase: Phase,
        _overlay: &[(std::path::PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        Ok(vec![Finding {
            rule: "x\u{1b}[31m".into(),
            count: usize::from(phase == Phase::Before),
        }])
    }
}

#[test]
fn a_rule_name_from_a_linter_is_escaped_in_the_report() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", DIRECTIVE);
    let m = command().get_matches_from(["migrate", dir.path().to_str().unwrap()]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let state = tempfile::tempdir().unwrap();
    let code = run(&m, &Hostile, STEERING, state.path(), &mut out, &mut err);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(out.contains("x\\u{1b}[31m"), "{out:?}");
}

#[test]
fn a_multiline_mdx_directive_is_reported_at_its_start_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.mdx",
        "# T\n\n<!--\nslopvac-allow: rule=prose-format.no-unicode-dash\nreason=x -->\n",
    );
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.changes.len(), 0);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(
        p.warnings[0].starts_with("a.mdx:3: MDX is not rewritten"),
        "{:?}",
        p.warnings
    );
}

#[test]
fn mdx_directives_that_no_rewrite_would_change_are_still_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.mdx",
        "<!-- slopvac-allow: rule=no.such-rule reason=x -->\n\n<!-- slopvac-allow: rule=a.b,c.d reason=x -->\n",
    );
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
    assert!(
        p.warnings[0].starts_with("a.mdx:1: MDX is not rewritten"),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings[1].starts_with("a.mdx:3: MDX is not rewritten"),
        "{:?}",
        p.warnings
    );
}

fn dropped_of(root: &Path) -> Vec<slopvac_migrate::Dropped> {
    match plan_of(root) {
        Err(Error::Dropped(d)) => d,
        other => panic!("expected Dropped, got {other:?}"),
    }
}

#[test]
fn blocking_records_are_capped_with_a_suppressed_count() {
    let dir = tempfile::tempdir().unwrap();
    let one = "<!-- slopvac-allow: rule=no.such-rule reason=x -->\n\n";
    write(dir.path(), "a.md", one.repeat(250));
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 101);
    assert_eq!(d[100].reason, "150 more blocking settings suppressed");
    assert_eq!(d[0].line, 1);
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(err.starts_with("error[config-legacy-dropped]: "), "{err}");
    assert!(
        err.trim_end()
            .ends_with("150 more blocking settings suppressed"),
        "{err}"
    );
}

#[test]
fn each_blocking_field_is_cut_to_its_length_cap() {
    let dir = tempfile::tempdir().unwrap();
    let id = format!("no.{}", "x".repeat(2000));
    write(
        dir.path(),
        "a.md",
        format!("<!-- slopvac-allow: rule={id} reason=x -->\n"),
    );
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].key.chars().count(), 513);
    assert!(d[0].key.ends_with('…'));
}

#[test]
fn a_read_failure_keeps_its_filesystem_and_io_source_chain() {
    use std::error::Error as _;
    let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let e = Error::Read {
        path: "a.md".into(),
        source: slopvac_fs::Error::read(&io),
    };
    let fs_error = e.source().expect("the filesystem error");
    let io_error = fs_error.source().expect("the io error");
    let io_error = io_error
        .downcast_ref::<std::io::Error>()
        .expect("an io error");
    assert_eq!(io_error.kind(), std::io::ErrorKind::PermissionDenied);
}

fn unknown_keys(count: usize, prefix: &str) -> String {
    (0..count)
        .map(|i| [prefix, &i.to_string(), " = 1\n"].concat())
        .collect()
}

#[test]
fn config_blockers_are_capped_while_the_config_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", unknown_keys(250, "bogus"));
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 101);
    assert_eq!(d[0].key, "bogus0");
    assert_eq!(d[0].file, "slopvac.toml");
    assert_eq!(d[0].suppressed, 0);
    assert_eq!(d[100].suppressed, 150);
    assert_eq!(d[100].reason, "150 more blocking settings suppressed");
    assert_eq!(cli(dir.path()).0, 2);
}

#[test]
fn suppressed_counts_add_up_across_config_files() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", unknown_keys(60, "a"));
    write(dir.path(), "sub/slopvac.toml", unknown_keys(60, "b"));
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 101);
    assert_eq!(d[100].suppressed, 20);
}

#[test]
fn an_empty_top_level_key_is_a_real_record_not_the_suppression_marker() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "\"\" = 1\n");
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].file, "slopvac.toml");
    assert_eq!(d[0].key, "");
    assert_eq!(d[0].suppressed, 0);
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(err.contains("slopvac.toml:"), "{err}");
}

fn migrate_text(text: &str, preserve_behavior: bool) -> Vec<slopvac_migrate::Dropped> {
    match migrate_config(
        text,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options { preserve_behavior },
    ) {
        Err(Error::Dropped(d)) => d,
        other => panic!("expected Dropped, got {other:?}"),
    }
}

#[test]
fn migrate_config_bounds_its_blockers_and_counts_the_rest() {
    let d = migrate_text(&unknown_keys(250, "bogus"), false);
    assert_eq!(d.len(), 101);
    assert!(d[..100].iter().all(|r| r.suppressed == 0));
    assert_eq!(d[100].suppressed, 150);
}

#[test]
fn migrate_config_bounds_its_preserve_behavior_blockers_and_counts_the_rest() {
    let block = "[[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"lax\"\n";
    let d = migrate_text(&block.repeat(150), true);
    assert_eq!(d.len(), 101);
    assert!(d[..100].iter().all(|r| r.suppressed == 0));
    assert_eq!(d[100].suppressed, 50);
}

#[test]
fn a_key_over_the_display_cap_keeps_its_pyproject_line() {
    let dir = tempfile::tempdir().unwrap();
    let key = "k".repeat(600);
    write(
        dir.path(),
        "pyproject.toml",
        format!("[project]\nname = \"x\"\n\n[tool.slopvac]\nprofile = \"normal\"\n\"{key}\" = 1\n"),
    );
    let d = dropped_of(dir.path());
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].file, "pyproject.toml");
    assert_eq!(d[0].line, 6);
    assert_eq!(d[0].key.chars().count(), 513);
}
