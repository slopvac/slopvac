//! Project-level planning, apply, restore and the command.

use std::fs;
use std::path::Path;

use slopvac_migrate::{
    Error, Finding, Linter, NullLinter, Options, Phase, RenameTable, apply, command, plan,
    render_diff, restore, run,
};

fn write(root: &Path, name: &str, text: &str) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn plan_of(root: &Path) -> Result<slopvac_migrate::Plan, Error> {
    plan(root, &RenameTable::bundled(), Options::default())
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
        "<!-- slopvac-allow: rule=a.b reason=x -->\n",
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
        "<!-- slopvac-allow: rule=a.b reason=x -->\n",
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
        "<!-- slopvac-allow: rule=a.b reason=x -->\n"
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
        "<!-- slopvac-allow: rule=a.b reason=x -->\n",
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
                rule: "a.x".into(),
                count: 3,
            }],
            Phase::After => {
                assert_eq!(overlay.len(), 1);
                vec![Finding {
                    rule: "core:a.x".into(),
                    count: 1,
                }]
            }
        })
    }
}

fn run_cli(args: &[&str], linter: &dyn Linter) -> (i32, String, String) {
    let m = command().get_matches_from(std::iter::once("migrate").chain(args.iter().copied()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&m, linter, &mut out, &mut err);
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
    assert!(out.contains("core:a.x: 3 -> 1"), "{out}");
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
        err.contains("config-legacy-dropped: slopvac.toml:1: `bogus`"),
        "{err}"
    );
    assert!(err.contains("slopvac.toml:3: `vale.enabled`"), "{err}");
}
