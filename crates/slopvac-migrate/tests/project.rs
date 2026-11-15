//! Project-level planning, apply, restore and the command.

use std::fs;
use std::path::Path;

use slopvac_migrate::{
    Error, Finding, Linter, NullLinter, Options, Phase, RenameTable, apply, command, plan,
    render_diff, restore, run,
};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";

fn write(root: &Path, name: &str, text: &str) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn plan_of(root: &Path) -> Result<slopvac_migrate::Plan, Error> {
    plan(root, &RenameTable::bundled(), Options::default(), STEERING)
}

#[test]
fn dotfile_becomes_slopvac_toml() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".slopvac.toml", "profile = \"strict\"\n");
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    assert!(!dir.path().join(".slopvac.toml").exists());
    let new = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    assert_eq!(new, "profile = \"strict\"\nschema_version = 3\n");
}

#[test]
fn pyproject_table_moves_and_other_content_stays() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\" # keep\n\n[tool.ruff]\nline-length = 88\n\n[tool.slopvac]\nprofile = \"normal\"\n[tool.slopvac.thresholds]\nmax_errors = 1\n",
    );
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    let py = fs::read_to_string(dir.path().join("pyproject.toml")).unwrap();
    assert!(py.contains("name = \"x\" # keep"), "{py}");
    assert!(py.contains("line-length = 88"), "{py}");
    assert!(!py.contains("slopvac"), "{py}");
    let new = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    assert!(new.contains("profile = \"normal\""), "{new}");
    assert!(new.contains("[thresholds]\nmax_errors = 1"), "{new}");
    assert!(new.contains("schema_version = 3"), "{new}");
}

#[test]
fn pyproject_drop_reports_the_pyproject_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.slopvac]\nprofile = \"normal\"\nbogus = 1\n",
    );
    match plan_of(dir.path()) {
        Err(Error::Dropped(d)) => {
            assert_eq!(d.len(), 1);
            assert_eq!((d[0].key.as_str(), d[0].line), ("bogus", 6));
            assert_eq!(d[0].file, "pyproject.toml");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pyproject_overlap_error_reports_the_pyproject_line() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.slopvac]\nprofile = \"normal\"\n\n\
         [[tool.slopvac.overrides]]\nfiles = [\"a.md\"]\n",
    );
    let options = Options {
        preserve_behavior: true,
    };
    match plan(dir.path(), &RenameTable::bundled(), options, STEERING) {
        Err(Error::OverrideOverlap {
            file, line, block, ..
        }) => assert_eq!((file.as_str(), line, block), ("pyproject.toml", 7, 0)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn two_config_sources_in_one_directory_conflict() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    write(dir.path(), ".slopvac.toml", "profile = \"normal\"\n");
    assert!(matches!(plan_of(dir.path()), Err(Error::Conflict(_))));
}

#[test]
fn nested_configs_each_migrate() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    write(dir.path(), "docs/slopvac.toml", "profile = \"normal\"\n");
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.changes.len(), 2);
}

#[test]
fn documents_get_directive_ids_and_steering_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "AGENTS.md",
        "# Agents\n\n<!-- slopvac:begin -->\nold text\n<!-- slopvac:end -->\n",
    );
    write(
        dir.path(),
        "docs/a.md",
        "<!-- slopvac-allow: rule=prose-format.no-unicode-dash reason=quotation -->\nText\n",
    );
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    let agents = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
    assert!(!agents.contains("old text"));
    let a = fs::read_to_string(dir.path().join("docs/a.md")).unwrap();
    assert!(a.contains("rule=core:prose-format.no-unicode-dash"));
}

#[test]
fn second_run_finds_nothing_to_do() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    write(
        dir.path(),
        "a.md",
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n",
    );
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    assert!(plan_of(dir.path()).unwrap().is_empty());
}

#[test]
fn restore_undoes_apply() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".slopvac.toml", "profile = \"strict\"\n");
    write(
        dir.path(),
        "a.md",
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n",
    );
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    assert_eq!(restore(dir.path()).unwrap(), 3);
    assert_eq!(
        fs::read_to_string(dir.path().join(".slopvac.toml")).unwrap(),
        "profile = \"strict\"\n"
    );
    assert!(!dir.path().join("slopvac.toml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n"
    );
    assert!(!dir.path().join(slopvac_migrate::BACKUP_DIR).exists());
}

#[test]
fn a_dropped_setting_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "[vale]\nenabled = true\n");
    write(
        dir.path(),
        "a.md",
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n",
    );
    assert!(matches!(plan_of(dir.path()), Err(Error::Dropped(_))));
}

#[test]
fn diff_names_files_and_marks_new_ones() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".slopvac.toml", "profile = \"strict\"\n");
    let diff = render_diff(&plan_of(dir.path()).unwrap());
    assert!(diff.contains("--- /dev/null\n+++ b/slopvac.toml"), "{diff}");
    assert!(
        diff.contains("--- a/.slopvac.toml\n+++ /dev/null"),
        "{diff}"
    );
}

struct FakeLinter;

impl Linter for FakeLinter {
    fn lint(
        &self,
        _root: &Path,
        phase: Phase,
        overlay: &[(std::path::PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        Ok(match phase {
            Phase::Before => vec![Finding {
                rule: "orwell.not-un".into(),
                count: 3,
            }],
            Phase::After => {
                assert_eq!(overlay.len(), 1);
                vec![Finding {
                    rule: "core:orwell.not-un".into(),
                    count: 1,
                }]
            }
        })
    }
}

fn run_cli(args: &[&str], linter: &dyn Linter) -> (i32, String, String) {
    let m = command().get_matches_from(std::iter::once("migrate").chain(args.iter().copied()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&m, linter, STEERING, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn cli_previews_by_default_and_reports_rule_changes() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    let path = dir.path().to_str().unwrap();
    let (code, out, _) = run_cli(&[path], &FakeLinter);
    assert_eq!(code, 0);
    assert!(out.contains("core:orwell.not-un: 3 -> 1"), "{out}");
    assert!(out.contains("Preview only"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).unwrap(),
        "profile = \"strict\"\n"
    );
}

#[test]
fn cli_write_applies() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    let path = dir.path().to_str().unwrap();
    let (code, _, _) = run_cli(&[path, "--write"], &NullLinter);
    assert_eq!(code, 0);
    assert!(
        fs::read_to_string(dir.path().join("slopvac.toml"))
            .unwrap()
            .contains("schema_version = 3")
    );
}

#[test]
fn cli_exits_2_and_names_each_dropped_setting() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "slopvac.toml",
        "bogus = 1\n[vale]\nenabled = true\n",
    );
    let path = dir.path().to_str().unwrap();
    let (code, _, err) = run_cli(&[path, "--write"], &NullLinter);
    assert_eq!(code, 2);
    assert!(
        err.contains("error[config-legacy-dropped]: slopvac.toml:1: `bogus`"),
        "{err}"
    );
    assert!(err.contains("slopvac.toml:3: `vale.enabled`"), "{err}");
}

fn backup_with(dir: &Path, manifest: &str) {
    let b = dir.join(slopvac_migrate::BACKUP_DIR);
    fs::create_dir_all(&b).unwrap();
    fs::write(b.join("manifest.txt"), manifest).unwrap();
}

#[test]
fn restore_rejects_paths_that_leave_the_project() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    fs::write(outer.path().join("victim.txt"), "keep").unwrap();
    let abs = format!("C {}\n", outer.path().join("victim.txt").display());
    for manifest in [
        "C ../victim.txt\n",
        abs.as_str(),
        "M ../victim.txt\n",
        "C a/../../victim.txt\n",
    ] {
        backup_with(&root, manifest);
        assert!(
            matches!(restore(&root), Err(Error::Unsafe(_))),
            "{manifest}"
        );
        assert_eq!(
            fs::read_to_string(outer.path().join("victim.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(root.join(slopvac_migrate::BACKUP_DIR)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn restore_rejects_a_symlinked_directory() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("proj");
    let elsewhere = outer.path().join("elsewhere");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("victim.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("link")).unwrap();
    backup_with(&root, "C link/victim.txt\n");
    assert!(matches!(restore(&root), Err(Error::Unsafe(_))));
    assert!(elsewhere.join("victim.txt").exists());
}

#[test]
fn restore_checks_every_entry_before_touching_any_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "made.toml", "new");
    backup_with(dir.path(), "C made.toml\nC ../escape\n");
    assert!(restore(dir.path()).is_err());
    assert!(dir.path().join("made.toml").exists());
}

#[test]
fn apply_refuses_while_a_backup_exists() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"normal\"\n");
    assert!(matches!(apply(dir.path(), &p), Err(Error::Unsafe(_))));
}

#[test]
fn inline_tool_slopvac_table_migrates() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool]\nruff = { line-length = 88 }\nslopvac = { profile = \"strict\", exclude = [\"a\"] }\n",
    );
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    let py = fs::read_to_string(dir.path().join("pyproject.toml")).unwrap();
    assert!(py.contains("ruff = { line-length = 88 }"), "{py}");
    assert!(!py.contains("slopvac"), "{py}");
    let new = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    assert!(new.contains("profile = \"strict\""), "{new}");
    assert!(new.contains("exclude = [\"a\"]"), "{new}");
}

#[test]
fn dotted_tool_slopvac_keys_migrate() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[tool]\nslopvac.profile = \"strict\"\n",
    );
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    let new = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    assert!(new.contains("profile = \"strict\""), "{new}");
    let py = fs::read_to_string(dir.path().join("pyproject.toml")).unwrap();
    assert!(!py.contains("slopvac"), "{py}");
}

#[test]
fn an_unmapped_directive_id_stops_the_command_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = "profile = \"strict\"\n";
    let doc = "# Title\n\n<!-- slopvac-allow: rule=prose-craft.no-such-rule reason=x -->\nText\n";
    write(dir.path(), "slopvac.toml", config);
    write(dir.path(), "docs/a.md", doc);
    let (code, _, err) = run_cli(&[dir.path().to_str().unwrap(), "--write"], &NullLinter);
    assert_eq!(code, 2, "{err}");
    assert!(err.starts_with("error[config-legacy-dropped]: "), "{err}");
    assert!(
        err.contains("docs/a.md:3: `slopvac-allow rule=prose-craft.no-such-rule`"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("docs/a.md")).unwrap(),
        doc
    );
    assert!(!dir.path().join(slopvac_migrate::BACKUP_DIR).exists());
}

#[test]
fn unmapped_directive_ids_and_dropped_settings_come_back_together() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "[vale]\nenabled = true\n");
    write(
        dir.path(),
        "a.md",
        "<!-- slopvac-allow: rule=orwell.no-such-rule reason=x -->\n",
    );
    match plan_of(dir.path()) {
        Err(Error::Dropped(d)) => {
            let files: Vec<_> = d.iter().map(|x| x.file.as_str()).collect();
            assert!(files.contains(&"slopvac.toml"), "{d:?}");
            assert!(files.contains(&"a.md"), "{d:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn warnings_print_even_when_there_is_nothing_to_change() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "<!-- slopvac:begin -->\nunterminated\n");
    let (code, out, _) = run_cli(&[dir.path().to_str().unwrap()], &NullLinter);
    assert_eq!(code, 0);
    assert!(
        out.contains("warning: a.md: malformed or ambiguous"),
        "{out}"
    );
    assert!(out.contains("Nothing to migrate."), "{out}");
}

#[test]
fn schema_version_two_config_migrates_through_the_command() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "slopvac.toml",
        "schema_version = 2\nprofile = \"strict\"\n",
    );
    let (code, _, err) = run_cli(&[dir.path().to_str().unwrap(), "--write"], &NullLinter);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).unwrap(),
        "schema_version = 3\nprofile = \"strict\"\n"
    );
}

#[test]
fn errors_carry_codes_and_no_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-project");
    let (code, _, err) = run_cli(&[missing.to_str().unwrap()], &NullLinter);
    assert_eq!(code, 2);
    assert!(err.starts_with("error[migrate-io-failed]: "), "{err}");
    assert!(!err.contains(dir.path().to_str().unwrap()), "{err}");
}
