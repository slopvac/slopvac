//! Reading, discovery and terminal output: escapes are neutralised, caps and run limits
//! stop the run, unreadable input is exit 2, and unsupported files are reported.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use slopvac_migrate::{
    Error, Limits, NullLinter, Options, RenameTable, command, plan, plan_with_limits, run,
};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";
const ESC_NAME: &str = "\u{1b}[31mred";
const DIRECTIVE: &str =
    "<!-- slopvac-allow: rule=prose-format.no-unicode-dash reason=x -->\nText\n";

fn write(root: &Path, name: &str, text: impl AsRef<[u8]>) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
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

fn plan_of(root: &Path) -> Result<slopvac_migrate::Plan, Error> {
    plan(root, &RenameTable::bundled(), Options::default(), STEERING)
}

fn sparse(path: &Path, len: u64) {
    fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn the_diff_neutralises_escapes_in_file_names_and_bodies() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &format!("docs/{ESC_NAME}.md"),
        format!("\u{1b}[2Jbody\n{DIRECTIVE}"),
    );
    let (code, out, _) = cli(dir.path());
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(out.contains("\\u{1b}[31mred.md"), "{out:?}");
    assert!(out.contains("\\u{1b}[2Jbody"), "{out:?}");
}

#[test]
fn notes_and_warnings_neutralise_escapes() {
    let dir = tempfile::tempdir().unwrap();
    let removed =
        "<!-- slopvac-allow: rule=ste-words.inconsistent-term-for-same-thing reason=x -->\n";
    let invalid = "<!-- slopvac-allow: rule=a.b,c.d reason=x -->\n";
    write(
        dir.path(),
        &format!("docs/{ESC_NAME}.md"),
        format!("{removed}{invalid}"),
    );
    let (code, out, _) = cli(dir.path());
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(
        out.contains("note: migrate-rule-removed: docs/\\u{1b}[31mred.md:1:"),
        "{out:?}"
    );
    assert!(
        out.contains("warning: docs/\\u{1b}[31mred.md:2:"),
        "{out:?}"
    );
}

#[test]
fn errors_neutralise_escapes() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &format!("docs/{ESC_NAME}.md"),
        "<!-- slopvac-allow: rule=no.such-rule reason=x -->\n",
    );
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(!err.contains('\u{1b}'), "{err:?}");
    assert!(err.contains("docs/\\u{1b}[31mred.md:1:"), "{err:?}");
}

#[test]
fn a_document_over_the_cap_is_exit_2_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("docs")).unwrap();
    sparse(&dir.path().join("docs/big.md"), 64 * 1024 * 1024 + 1);
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[document-too-large]: docs/big.md: "),
        "{err}"
    );
}

#[test]
fn a_config_over_the_cap_is_exit_2_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    sparse(&dir.path().join("slopvac.toml"), 1024 * 1024 + 1);
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[config-too-large]: slopvac.toml: "),
        "{err}"
    );
}

#[test]
fn a_file_at_the_cap_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let pad = "#".repeat(1024 * 1024 - "schema_version = 3\n".len());
    write(
        dir.path(),
        "slopvac.toml",
        format!("schema_version = 3\n{pad}"),
    );
    let (code, out, err) = cli(dir.path());
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
}

#[test]
fn more_files_than_the_run_limit_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "x\n");
    write(dir.path(), "b.md", "y\n");
    let limits = Limits::new(1, u64::MAX);
    let e = plan_with_limits(
        dir.path(),
        &RenameTable::bundled(),
        Options::default(),
        STEERING,
        limits,
    )
    .unwrap_err();
    assert_eq!(e.code(), "migrate-run-limit");
    assert!(e.to_string().contains(".md: run limit"), "{e}");
}

#[test]
fn more_bytes_than_the_run_limit_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "123\n");
    write(dir.path(), "b.md", "456\n");
    let e = plan_with_limits(
        dir.path(),
        &RenameTable::bundled(),
        Options::default(),
        STEERING,
        Limits::new(10, 6),
    )
    .unwrap_err();
    assert_eq!(e.code(), "migrate-run-limit");
    assert!(e.to_string().contains(".md: run limit"), "{e}");
}

#[test]
fn a_document_that_is_not_utf8_is_exit_2_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "docs/bad.md", [0xff, 0xfe, b'\n']);
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[migrate-not-utf8]: docs/bad.md: "),
        "{err}"
    );
}

#[test]
fn a_malformed_pyproject_is_exit_2_with_file_and_line() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "pyproject.toml", "[project]\nname = \n");
    let (code, _, err) = cli(dir.path());
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[config-parse]: pyproject.toml: line 2: "),
        "{err}"
    );
}

#[test]
fn markdown_and_mdx_are_found_whatever_the_extension_case() {
    for name in ["a.MD", "a.markdown", "A.Markdown"] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), name, DIRECTIVE);
        let p = plan_of(dir.path()).unwrap();
        let paths: Vec<_> = p.changes.iter().map(|c| c.path.to_str().unwrap()).collect();
        assert_eq!(paths, [name], "{name}");
    }
}

#[test]
fn a_prose_file_migrate_cannot_rewrite_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes.txt", DIRECTIVE);
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.changes.len(), 0);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(
        p.warnings[0].starts_with("notes.txt: not checked: "),
        "{:?}",
        p.warnings
    );
}
