//! Project-level planning, apply, restore and the command.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;

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

/// A per-user cache directory for the backup record, apart from the project.
fn cache() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn plan_of(root: &Path) -> Result<slopvac_migrate::Plan, Error> {
    plan(root, &RenameTable::bundled(), Options::default(), STEERING)
}

#[test]
fn dotfile_becomes_slopvac_toml() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".slopvac.toml", "profile = \"strict\"\n");
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p, cache().path()).unwrap();
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
    apply(dir.path(), &p, cache().path()).unwrap();
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

#[rstest]
#[case::plain(false)]
#[case::preserve_behavior(true)]
fn pyproject_removed_category_notices_report_the_pyproject_lines(#[case] preserve_behavior: bool) {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.slopvac]\nprofile = \"normal\"\n\
         [tool.slopvac.categories]\norwell-voice = \"off\"\n\n\
         [[tool.slopvac.overrides]]\nfiles = [\"docs/**\"]\n\
         [tool.slopvac.overrides.categories]\norwell-voice = \"error\"\n",
    );
    let options = Options { preserve_behavior };
    let p = plan(dir.path(), &RenameTable::bundled(), options, STEERING).unwrap();
    let notices: Vec<&String> = p
        .notes
        .iter()
        .filter(|n| n.contains("migrate-category-removed"))
        .collect();
    assert_eq!(
        notices,
        [
            "pyproject.toml: migrate-category-removed: pyproject.toml:7: `categories.orwell-voice`: category no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml",
            "pyproject.toml: migrate-category-removed: pyproject.toml:12: `overrides[0].categories.orwell-voice`: category no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml",
        ]
    );
}

#[test]
fn pyproject_removed_rule_is_dropped_and_its_notice_reports_the_pyproject_line() {
    let id = "ste-words.inconsistent-term-for-same-thing";
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        &format!(
            "[project]\nname = \"x\"\n\n[tool.slopvac]\nprofile = \"normal\"\n[tool.slopvac.rules]\n\"{id}\" = \"error\"\n"
        ),
    );
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p, cache().path()).unwrap();
    let new = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    assert!(!new.contains(id), "{new}");
    let notices: Vec<&String> = p
        .notes
        .iter()
        .filter(|n| n.contains("migrate-rule-removed"))
        .collect();
    assert_eq!(
        notices,
        [&format!(
            "pyproject.toml: migrate-rule-removed: pyproject.toml:7: `rules.{id}`: rule no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
        )]
    );
}

#[test]
fn two_config_sources_in_one_directory_conflict() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    write(dir.path(), ".slopvac.toml", "profile = \"normal\"\n");
    assert!(matches!(plan_of(dir.path()), Err(Error::Conflict(_))));
}

#[rstest]
#[case::plain(false)]
#[case::preserve_behavior(true)]
fn a_migrated_config_the_3_0_loader_refuses_stops_the_plan(#[case] preserve_behavior: bool) {
    let dir = tempfile::tempdir().unwrap();
    // 2.x read both blocks; 3.0 refuses two override blocks with the same files.
    write(
        dir.path(),
        "slopvac.toml",
        "[[overrides]]\nfiles = [\"docs/**\"]\nthresholds = { max_errors = 1 }\n\n\
         [[overrides]]\nfiles = [\"docs/**\"]\nlocale = { default = \"en-GB\" }\n",
    );
    let err = plan(
        dir.path(),
        &RenameTable::bundled(),
        Options { preserve_behavior },
        STEERING,
    )
    .unwrap_err();
    assert_eq!(err.code(), "migrate-output-invalid", "{err}");
    assert!(err.to_string().contains("config-duplicate-scope"), "{err}");
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
    apply(dir.path(), &p, cache().path()).unwrap();
    let agents = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
    assert!(!agents.contains("old text"));
    let a = fs::read_to_string(dir.path().join("docs/a.md")).unwrap();
    assert!(a.contains("rule=core:prose-format.no-unicode-dash"));
}

#[test]
fn directives_for_removed_rules_leave_an_empty_comment_and_a_note() {
    for id in [
        "ste-practices.inconsistent-wording-for-same-step",
        "ste-safety.safety-block-missing-consequence",
        "ste-sentences.vertical-list-item-punctuation",
        "ste-sentences.vertical-list-lead-in-missing-colon",
        "ste-words.inconsistent-term-for-same-thing",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "docs/a.md",
            &format!("<!-- slopvac-allow: rule={id} reason=x -->\nText\n"),
        );
        let p = plan_of(dir.path()).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        apply(dir.path(), &p, cache().path()).unwrap();
        let a = fs::read_to_string(dir.path().join("docs/a.md")).unwrap();
        assert_eq!(a, "<!-- -->\nText\n", "{id}");
        assert_eq!(
            p.notes,
            vec![format!(
                "migrate-rule-removed: docs/a.md:1: `slopvac-allow rule={id}`: rule no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
            )]
        );
    }
}
#[test]
fn a_comma_list_directive_is_left_alone_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let text = "Intro\n<!-- slopvac-allow: rule=ste-sentences.vertical-list-item-punctuation,prose-craft.latinisms reason=x -->\nText\n";
    write(dir.path(), "docs/a.md", text);
    let p = plan_of(dir.path()).unwrap();
    assert_eq!(p.changes.len(), 0);
    assert_eq!(
        p.warnings,
        vec!["docs/a.md:2: invalid 2.x directive, left as is".to_owned()]
    );
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
    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache().path()).unwrap();
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
    let cache = cache();
    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache.path()).unwrap();
    let done = restore(dir.path(), cache.path(), false).unwrap();
    assert_eq!((done.restored, done.kept.len()), (3, 0));
    assert_eq!(
        fs::read_to_string(dir.path().join(".slopvac.toml")).unwrap(),
        "profile = \"strict\"\n"
    );
    assert!(!dir.path().join("slopvac.toml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n"
    );
    assert!(!dir.path().join(".slopvac").exists());
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
    let cache = cache();
    let code = run(&m, linter, STEERING, cache.path(), &mut out, &mut err);
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
fn cli_labels_the_rule_table_as_the_3_0_engine_on_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    let path = dir.path().to_str().unwrap();
    let (_, out, _) = run_cli(&[path], &FakeLinter);
    assert!(
        out.contains(
            "Findings by rule, from the 3.0 engine on your files before and after migration \
             (before -> after):\n  core:orwell.not-un: 3 -> 1\n"
        ),
        "{out}"
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

#[test]
fn apply_rejects_paths_that_leave_the_project() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    fs::write(outer.path().join("victim.txt"), "keep").unwrap();
    let abs = outer.path().join("victim.txt");
    for path in [
        Path::new("../victim.txt"),
        abs.as_path(),
        Path::new("a/../../victim.txt"),
    ] {
        let p = slopvac_migrate::Plan {
            changes: vec![slopvac_migrate::Change {
                path: path.to_owned(),
                kind: slopvac_migrate::ChangeKind::Write,
                old: Some("keep".into()),
                new: Some("gone".into()),
            }],
            ..slopvac_migrate::Plan::default()
        };
        assert!(
            matches!(apply(&root, &p, cache().path()), Err(Error::Unsafe(_))),
            "{}",
            path.display()
        );
        assert_eq!(
            fs::read_to_string(outer.path().join("victim.txt")).unwrap(),
            "keep"
        );
        assert!(!root.join(".slopvac").exists());
    }
}

#[cfg(unix)]
#[test]
fn restore_rejects_a_symlinked_directory() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("proj");
    let elsewhere = outer.path().join("elsewhere");
    write(
        &root,
        "docs/a.md",
        "<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n",
    );
    let cache = cache();
    apply(&root, &plan_of(&root).unwrap(), cache.path()).unwrap();
    // The migrated directory moves out of the project and a link to it takes its place.
    fs::rename(root.join("docs"), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("docs")).unwrap();
    let migrated = fs::read_to_string(elsewhere.join("a.md")).unwrap();
    assert!(matches!(
        restore(&root, cache.path(), true),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(
        fs::read_to_string(elsewhere.join("a.md")).unwrap(),
        migrated
    );
}

/// The names in the project's backup directory, sorted. Empty when it does not exist.
fn backups(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root.join(".slopvac/migrate-backup")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn apply_refuses_while_a_backup_exists() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache();
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p, cache.path()).unwrap();
    let first = backups(dir.path());
    assert_eq!(first.len(), 1, "{first:?}");
    write(dir.path(), "slopvac.toml", "profile = \"normal\"\n");
    assert!(matches!(
        apply(dir.path(), &p, cache.path()),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(backups(dir.path()), first);
}

#[test]
fn a_backup_directory_no_record_names_does_not_block_apply_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache();
    write(dir.path(), ".slopvac.toml", "profile = \"strict\"\n");
    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache.path()).unwrap();
    restore(dir.path(), cache.path(), false).unwrap();
    // A backup left over from an earlier run that was discarded by hand.
    let old = ".slopvac/migrate-backup/20200101T000000Z";
    write(dir.path(), &format!("{old}/0"), "earlier\n");

    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache.path()).unwrap();

    let names = backups(dir.path());
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(names[0], "20200101T000000Z");
    assert_eq!(
        fs::read_to_string(dir.path().join(old).join("0")).unwrap(),
        "earlier\n"
    );
    restore(dir.path(), cache.path(), false).unwrap();
    assert_eq!(backups(dir.path()), ["20200101T000000Z"]);
}

#[test]
fn inline_tool_slopvac_table_migrates() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool]\nruff = { line-length = 88 }\nslopvac = { profile = \"strict\", exclude = [\"a\"] }\n",
    );
    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache().path()).unwrap();
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
    apply(dir.path(), &plan_of(dir.path()).unwrap(), cache().path()).unwrap();
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
    // Diagnostics show project-relative paths with the platform separator, as the
    // config labels do (`rel(..).display()` in plan.rs).
    let shown = Path::new("docs").join("a.md").display().to_string();
    assert!(
        err.contains(&format!(
            "{shown}:3: `slopvac-allow rule=prose-craft.no-such-rule`"
        )),
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
    assert!(!dir.path().join(".slopvac").exists());
}

#[test]
fn a_directive_over_more_than_one_line_with_an_unmapped_id_is_a_warning_not_a_stop() {
    let dir = tempfile::tempdir().unwrap();
    let doc = "# Title\n\n<!-- slopvac-allow: rule=prose-craft.no-such-rule reason=x\n-->\nText\n";
    write(dir.path(), "slopvac.toml", "profile = \"strict\"\n");
    write(dir.path(), "docs/a.md", doc);
    let (code, out, err) = run_cli(&[dir.path().to_str().unwrap(), "--write"], &NullLinter);
    assert_eq!(code, 0, "{err}");
    let shown = Path::new("docs").join("a.md").display().to_string();
    assert!(
        out.contains(&format!(
            "warning: {shown}:3: invalid 2.x directive, left as is\n"
        )),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("docs/a.md")).unwrap(),
        doc
    );
    assert!(
        fs::read_to_string(dir.path().join("slopvac.toml"))
            .unwrap()
            .contains("schema_version = 3")
    );
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
