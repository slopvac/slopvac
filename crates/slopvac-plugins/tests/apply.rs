//! Safe application of fixer edits and recipe plans: threats O6 to O9 of
//! `docs/contract/threat-model.md`. Test names cite the test ids TM-14 to TM-18
//! and H22 (`docs/contract/plugin-protocol.md` section 12.1).
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use slopvac_plugins::apply::{
    ApplyError, EntryStatus, HarnessWarning, PlanOptions, apply_edits, apply_edits_seam,
    apply_plan, apply_plan_seam, edited_bytes,
};
#[cfg(unix)]
use slopvac_plugins::apply::{SeamStage, WriteOutcome};
use slopvac_plugins::hooks::{FixEdit, MAX_INSERTED_BYTES, RecipeAction, RecipeFile, Span};
use support::Scratch;

fn edit(start: u64, end: u64, text: &str) -> FixEdit {
    FixEdit {
        finding: 0,
        span: Span { start, end },
        text: text.into(),
    }
}

fn file(path: &str, action: RecipeAction, content: Option<&str>) -> RecipeFile {
    RecipeFile {
        path: path.into(),
        action,
        content: content.map(Into::into),
    }
}

fn write_file(path: &str, content: &str) -> RecipeFile {
    file(path, RecipeAction::Write, Some(content))
}

fn approve(paths: &[&str]) -> PlanOptions {
    PlanOptions {
        dry_run: false,
        allow_sensitive: paths.iter().map(|p| (*p).into()).collect(),
    }
}

fn dry() -> PlanOptions {
    PlanOptions {
        dry_run: true,
        allow_sensitive: vec![],
    }
}

/// A project root and a sibling directory outside it.
struct Sandbox {
    _tmp: Scratch,
    root: std::path::PathBuf,
    outside: std::path::PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = support::scratch();
    let root = tmp.path().join("project");
    let outside = tmp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    Sandbox {
        _tmp: tmp,
        root,
        outside,
    }
}

/// Names under `dir`, sorted, to prove nothing was created or left behind.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// The status of an entry that passed every check: planned, or refused on
/// Windows, where nothing is ever applied.
fn ok_status(path: &str) -> EntryStatus {
    if cfg!(windows) {
        EntryStatus::Refused(ApplyError::UnsupportedOnWindows(path.into()))
    } else {
        EntryStatus::Planned
    }
}

fn only_status(report: &slopvac_plugins::apply::PlanReport, i: usize) -> &EntryStatus {
    &report.entries[i].status
}

// ---- positions.md edit semantics (shared vectors) ----

fn vectors() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schemas/v3/conformance/positions/vectors.json"
    );
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn vector_edits(v: &Value) -> Vec<FixEdit> {
    v["edits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            edit(
                e["span"]["start"].as_u64().unwrap(),
                e["span"]["end"].as_u64().unwrap(),
                e["text"].as_str().unwrap(),
            )
        })
        .collect()
}

#[test]
fn positions_vectors_valid_edit_sets_give_the_pinned_result() {
    let v = vectors();
    let docs = &v["documents"];
    for case in v["edits"].as_array().unwrap() {
        let text = docs[case["document"].as_str().unwrap()]["text"]
            .as_str()
            .unwrap();
        let got = edited_bytes(text.as_bytes(), &vector_edits(case)).unwrap();
        assert_eq!(
            got,
            case["result"].as_str().unwrap().as_bytes(),
            "{}",
            case["id"]
        );
    }
}

#[test]
fn positions_vectors_invalid_edit_sets_are_rejected_whole() {
    let v = vectors();
    let docs = &v["documents"];
    for case in v["invalid_edits"].as_array().unwrap() {
        let text = docs[case["document"].as_str().unwrap()]["text"]
            .as_str()
            .unwrap();
        let got = edited_bytes(text.as_bytes(), &vector_edits(case));
        assert!(
            matches!(got, Err(ApplyError::InvalidEdits(_))),
            "{}",
            case["id"]
        );
    }
}

#[test]
fn edits_reject_a_source_that_is_not_utf8() {
    let got = edited_bytes(&[0xff, 0xfe], &[edit(0, 0, "x")]);
    assert!(matches!(got, Err(ApplyError::InvalidEdits(_))));
}

#[test]
fn tm14_edits_reject_more_than_the_edit_count_limit() {
    let many: Vec<FixEdit> = (0..1001).map(|_| edit(0, 0, "")).collect();
    assert!(matches!(
        edited_bytes(b"a", &many),
        Err(ApplyError::InvalidEdits(_))
    ));
}

// ---- apply_edits ----

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn apply_edits_rewrites_the_file_and_leaves_no_temporary_file() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab\ncd\n").unwrap();
    let r = apply_edits(&s.root, "a.md", b"ab\ncd\n", &[edit(3, 5, "CD")]).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Replaced);
    assert_eq!(r.edits, 1);
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab\nCD\n");
    assert_eq!(names(&s.root), ["a.md"]);
}

#[cfg(unix)]
#[test]
fn apply_edits_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let s = sandbox();
    let p = s.root.join("run.md");
    fs::write(&p, "x").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o750)).unwrap();
    apply_edits(&s.root, "run.md", b"x", &[edit(0, 1, "y")]).unwrap();
    assert_eq!(
        fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[test]
fn apply_edits_writes_nothing_when_the_edit_set_is_invalid() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab\ncd\n").unwrap();
    let bad = [edit(0, 3, "x"), edit(2, 4, "y")];
    let got = apply_edits(&s.root, "a.md", b"ab\ncd\n", &bad);
    assert!(matches!(got, Err(ApplyError::InvalidEdits(_))));
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab\ncd\n");
    assert_eq!(names(&s.root), ["a.md"]);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_edits_are_refused_when_the_file_changed_since_it_was_read() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "edited by someone else").unwrap();
    let got = apply_edits(&s.root, "a.md", b"ab\ncd\n", &[edit(0, 2, "AB")]);
    assert_eq!(got, Err(ApplyError::SourceChanged("a.md".into())));
    assert_eq!(
        fs::read_to_string(s.root.join("a.md")).unwrap(),
        "edited by someone else"
    );
    assert_eq!(names(&s.root), ["a.md"]);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_edits_are_refused_when_the_file_was_deleted() {
    let s = sandbox();
    let got = apply_edits(&s.root, "gone.md", b"x", &[edit(0, 1, "y")]);
    assert_eq!(got, Err(ApplyError::SourceChanged("gone.md".into())));
    assert_eq!(names(&s.root), Vec::<String>::new());
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_a_hardlinked_target_is_refused_and_both_names_keep_their_bytes() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab").unwrap();
    fs::hard_link(s.root.join("a.md"), s.root.join("b.md")).unwrap();
    let got = apply_edits(&s.root, "a.md", b"ab", &[edit(0, 2, "xy")]);
    assert_eq!(got, Err(ApplyError::Hardlinked("a.md".into())));
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab");
    assert_eq!(fs::read_to_string(s.root.join("b.md")).unwrap(), "ab");
    assert_eq!(names(&s.root), ["a.md", "b.md"]);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_a_directory_target_is_refused() {
    let s = sandbox();
    fs::create_dir(s.root.join("d.md")).unwrap();
    let got = apply_edits(&s.root, "d.md", b"x", &[edit(0, 1, "y")]);
    assert_eq!(got, Err(ApplyError::NotRegularFile("d.md".into())));
}

#[cfg(unix)]
#[test]
fn tm15_a_target_swapped_for_a_symlink_out_of_the_root_changes_nothing_outside() {
    let s = sandbox();
    let secret = s.outside.join("secret.txt");
    fs::write(&secret, "outside").unwrap();
    std::os::unix::fs::symlink(&secret, s.root.join("a.md")).unwrap();
    let got = apply_edits(&s.root, "a.md", b"outside", &[edit(0, 7, "pwned")]);
    assert!(matches!(got, Err(ApplyError::PathRefused(_))), "{got:?}");
    assert_eq!(fs::read_to_string(&secret).unwrap(), "outside");
}

#[cfg(unix)]
#[test]
fn tm15_a_target_swapped_for_a_symlink_inside_the_root_is_not_followed() {
    let s = sandbox();
    fs::write(s.root.join("real.md"), "real").unwrap();
    std::os::unix::fs::symlink("real.md", s.root.join("a.md")).unwrap();
    let got = apply_edits(&s.root, "a.md", b"real", &[edit(0, 4, "fake")]);
    assert_eq!(got, Err(ApplyError::Symlink("a.md".into())));
    assert_eq!(fs::read_to_string(s.root.join("real.md")).unwrap(), "real");
    assert!(
        fs::symlink_metadata(s.root.join("a.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn tm15_a_parent_swapped_for_a_symlink_out_of_the_root_changes_nothing_outside() {
    let s = sandbox();
    fs::write(s.outside.join("f.md"), "outside").unwrap();
    std::os::unix::fs::symlink(&s.outside, s.root.join("dir")).unwrap();
    let got = apply_edits(&s.root, "dir/f.md", b"outside", &[edit(0, 7, "pwned")]);
    assert!(matches!(got, Err(ApplyError::PathRefused(_))), "{got:?}");
    assert_eq!(
        fs::read_to_string(s.outside.join("f.md")).unwrap(),
        "outside"
    );
    assert_eq!(names(&s.outside), ["f.md"]);
}

#[cfg(unix)]
#[test]
fn tm15_a_parent_symlink_that_stays_inside_the_root_is_allowed() {
    let s = sandbox();
    fs::create_dir(s.root.join("real")).unwrap();
    fs::write(s.root.join("real/f.md"), "ab").unwrap();
    std::os::unix::fs::symlink("real", s.root.join("alias")).unwrap();
    apply_edits(&s.root, "alias/f.md", b"ab", &[edit(0, 1, "X")]).unwrap();
    assert_eq!(fs::read_to_string(s.root.join("real/f.md")).unwrap(), "Xb");
}

// ---- recipe plans ----

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn a_plan_creates_replaces_and_removes_files_under_the_root() {
    let s = sandbox();
    fs::write(s.root.join("old.md"), "old").unwrap();
    fs::write(s.root.join("gone.md"), "bye").unwrap();
    let plan = [
        write_file("docs/new/guide.md", "new"),
        write_file("old.md", "newer"),
        file("gone.md", RecipeAction::Remove, None),
        file("never-existed.md", RecipeAction::Remove, None),
        write_file("old.md.bak", ""),
    ];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    let statuses: Vec<_> = report.entries.iter().map(|e| e.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            EntryStatus::Done(WriteOutcome::Created),
            EntryStatus::Done(WriteOutcome::Replaced),
            EntryStatus::Done(WriteOutcome::Removed),
            EntryStatus::Done(WriteOutcome::Absent),
            EntryStatus::Done(WriteOutcome::Created),
        ]
    );
    assert!(report.wrote);
    assert_eq!(
        fs::read_to_string(s.root.join("docs/new/guide.md")).unwrap(),
        "new"
    );
    assert_eq!(fs::read_to_string(s.root.join("old.md")).unwrap(), "newer");
    assert_eq!(names(&s.root), ["docs", "old.md", "old.md.bak"]);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn a_plan_entry_with_identical_content_is_unchanged() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "same").unwrap();
    let report = apply_plan(
        &s.root,
        &[write_file("a.md", "same")],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Done(WriteOutcome::Unchanged)
    );
    assert!(!report.wrote);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn a_dry_run_checks_everything_and_writes_nothing() {
    let s = sandbox();
    let plan = [write_file("deep/er/a.md", "x"), write_file("b.md", "y")];
    let report = apply_plan(&s.root, &plan, &dry());
    assert!(
        report
            .entries
            .iter()
            .all(|e| e.status == EntryStatus::Planned)
    );
    assert!(!report.wrote);
    assert_eq!(names(&s.root), Vec::<String>::new());
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn merge_creates_a_missing_file_and_refuses_an_existing_one_until_markers_are_defined() {
    let s = sandbox();
    fs::write(s.root.join("have.md"), "mine").unwrap();
    let missing = apply_plan(
        &s.root,
        &[file("new.md", RecipeAction::Merge, Some("block"))],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&missing, 0),
        EntryStatus::Done(WriteOutcome::Created)
    );
    let existing = apply_plan(
        &s.root,
        &[file("have.md", RecipeAction::Merge, Some("block"))],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&existing, 0),
        EntryStatus::Refused(ApplyError::MergeUnsupported("have.md".into()))
    );
    assert_eq!(fs::read_to_string(s.root.join("have.md")).unwrap(), "mine");
}

#[test]
fn a_write_without_content_is_refused() {
    let s = sandbox();
    let report = apply_plan(
        &s.root,
        &[file("a.md", RecipeAction::Write, None)],
        &PlanOptions::default(),
    );
    assert!(matches!(
        only_status(&report, 0),
        EntryStatus::Refused(ApplyError::InvalidPlan(_))
    ));
    assert_eq!(names(&s.root), Vec::<String>::new());
}

#[test]
fn tm15_a_hardlinked_plan_target_is_refused_for_write_and_remove() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab").unwrap();
    fs::hard_link(s.root.join("a.md"), s.root.join("b.md")).unwrap();
    for f in [
        write_file("a.md", "new"),
        file("a.md", RecipeAction::Remove, None),
    ] {
        let report = apply_plan(&s.root, &[f], &PlanOptions::default());
        assert_eq!(
            *only_status(&report, 0),
            EntryStatus::Refused(ApplyError::Hardlinked("a.md".into()))
        );
    }
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab");
    assert_eq!(fs::read_to_string(s.root.join("b.md")).unwrap(), "ab");
}

#[cfg(unix)]
#[test]
fn tm15_a_plan_target_that_is_a_symlink_is_refused_for_write_and_remove() {
    let s = sandbox();
    fs::write(s.root.join("real.md"), "real").unwrap();
    std::os::unix::fs::symlink("real.md", s.root.join("a.md")).unwrap();
    for f in [
        write_file("a.md", "new"),
        file("a.md", RecipeAction::Remove, None),
    ] {
        let report = apply_plan(&s.root, &[f], &PlanOptions::default());
        assert_eq!(
            *only_status(&report, 0),
            EntryStatus::Refused(ApplyError::Symlink("a.md".into()))
        );
    }
    assert_eq!(fs::read_to_string(s.root.join("real.md")).unwrap(), "real");
}

#[test]
fn h22_an_absolute_path_and_a_dot_dot_path_are_refused_and_nothing_is_written() {
    let s = sandbox();
    let abs = s.outside.join("abs.md");
    let plan = [
        write_file("fine.md", "ok"),
        write_file(abs.to_str().unwrap(), "x"),
        write_file("../outside/up.md", "x"),
        write_file("a/../../outside/up2.md", "x"),
    ];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(*only_status(&report, 0), ok_status("fine.md"));
    for i in 1..4 {
        assert!(
            matches!(
                only_status(&report, i),
                EntryStatus::Refused(ApplyError::PathRefused(_))
            ),
            "entry {i}: {:?}",
            only_status(&report, i)
        );
    }
    assert!(!report.wrote);
    assert_eq!(names(&s.root), Vec::<String>::new());
    assert_eq!(names(&s.outside), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn h22_a_symlink_target_outside_the_root_is_refused_and_nothing_is_written() {
    let s = sandbox();
    let secret = s.outside.join("secret.txt");
    fs::write(&secret, "outside").unwrap();
    std::os::unix::fs::symlink(&secret, s.root.join("link.md")).unwrap();
    std::os::unix::fs::symlink(&s.outside, s.root.join("dir")).unwrap();
    let plan = [
        write_file("link.md", "pwned"),
        write_file("dir/new.md", "pwned"),
        write_file("fine.md", "ok"),
    ];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    for i in [0, 1] {
        assert!(
            matches!(
                only_status(&report, i),
                EntryStatus::Refused(ApplyError::PathRefused(_))
            ),
            "entry {i}: {:?}",
            only_status(&report, i)
        );
    }
    assert!(!report.wrote);
    assert_eq!(fs::read_to_string(&secret).unwrap(), "outside");
    assert_eq!(names(&s.outside), ["secret.txt"]);
    assert!(!s.root.join("fine.md").exists());
}

const DENIED: [&str; 16] = [
    ".git/config",
    ".git/hooks/pre-commit",
    ".slopvac/plugins.lock",
    ".slopvac/trust.toml",
    ".github/workflows/ci.yml",
    ".env",
    ".env.local",
    ".envrc",
    ".gitmodules",
    ".gitattributes",
    "slopvac.toml",
    ".mise.toml",
    ".vscode/tasks.json",
    "sub/.git/HEAD",
    ".GitHub/Workflows/CI.yml",
    "mise.toml",
];

#[test]
fn tm16_each_denied_path_is_refused_without_approval_and_nothing_is_written() {
    for denied in DENIED {
        let s = sandbox();
        let plan = [write_file(denied, "x"), write_file("fine.md", "ok")];
        let report = apply_plan(&s.root, &plan, &PlanOptions::default());
        let status = only_status(&report, 0);
        assert!(
            matches!(status, EntryStatus::Refused(ApplyError::Sensitive { .. })),
            "{denied}: {status:?}"
        );
        assert!(report.entries[0].sensitive.is_some(), "{denied}");
        assert!(!report.wrote, "{denied}");
        assert!(names(&s.root).is_empty(), "{denied}: {:?}", names(&s.root));
    }
}

#[rstest::rstest]
#[case::nested_state_dir("a/.slopvac/x")]
#[case::deeply_nested_state_dir("a/b/.slopvac/plugins.lock")]
#[case::root_dot_config(".slopvac.toml")]
#[case::nested_dot_config("a/.slopvac.toml")]
#[case::root_config("slopvac.toml")]
#[case::nested_config("a/slopvac.toml")]
#[case::mixed_case_state_dir(".SlopVac/x")]
#[case::mixed_case_nested_state_dir("a/.SlopVac/x")]
#[case::mixed_case_dot_config(".SLOPVAC.TOML")]
#[case::mixed_case_nested_dot_config("a/.SLOPVAC.TOML")]
#[case::mixed_case_nested_config("a/SlopVac.toml")]
#[case::long_s_state_dir("a/.\u{17f}lopvac/plugins.lock")]
#[case::long_s_root_state_dir(".\u{17f}lopvac/x")]
#[case::long_s_dot_config(".\u{17f}lopvac.toml")]
#[case::long_s_nested_config("a/\u{17f}lopvac.toml")]
fn tm16_slopvac_state_and_config_are_refused_at_any_depth(#[case] denied: &str) {
    let s = sandbox();
    let report = apply_plan(&s.root, &[write_file(denied, "x")], &PlanOptions::default());
    let status = only_status(&report, 0);
    assert!(
        matches!(status, EntryStatus::Refused(ApplyError::Sensitive { .. })),
        "{denied}: {status:?}"
    );
    assert!(!report.wrote, "{denied}");
    assert!(names(&s.root).is_empty(), "{denied}: {:?}", names(&s.root));
}

#[test]
fn tm16_workflows_are_refused_at_the_root_only() {
    let s = sandbox();
    let root = apply_plan(
        &s.root,
        &[write_file(".github/workflows/ci.yml", "x")],
        &dry(),
    );
    assert!(matches!(
        only_status(&root, 0),
        EntryStatus::Refused(ApplyError::Sensitive { .. })
    ));
    let nested = apply_plan(
        &s.root,
        &[write_file("a/.github/workflows/ci.yml", "x")],
        &dry(),
    );
    assert_eq!(
        *only_status(&nested, 0),
        ok_status("a/.github/workflows/ci.yml")
    );
}

#[test]
fn tm16_a_remove_of_a_denied_path_is_refused_too() {
    let s = sandbox();
    fs::write(s.root.join(".gitmodules"), "keep").unwrap();
    let report = apply_plan(
        &s.root,
        &[file(".gitmodules", RecipeAction::Remove, None)],
        &PlanOptions::default(),
    );
    assert!(matches!(
        only_status(&report, 0),
        EntryStatus::Refused(ApplyError::Sensitive { .. })
    ));
    assert_eq!(
        fs::read_to_string(s.root.join(".gitmodules")).unwrap(),
        "keep"
    );
}

#[test]
fn tm16_an_approval_covers_only_the_path_it_names() {
    let s = sandbox();
    let plan = [write_file(".env", "A=1"), write_file(".gitmodules", "m")];
    let report = apply_plan(&s.root, &plan, &approve(&[".ENV"]));
    assert!(matches!(
        only_status(&report, 1),
        EntryStatus::Refused(ApplyError::Sensitive { .. })
    ));
    assert_eq!(
        names(&s.root),
        Vec::<String>::new(),
        "plan is all or nothing"
    );
}

// Writes: Windows refuses these.
#[cfg(unix)]
#[test]
fn tm16_approving_every_denied_path_lets_the_plan_apply() {
    let s = sandbox();
    let plan = [write_file(".env", "A=1"), write_file(".gitmodules", "m")];
    let report = apply_plan(&s.root, &plan, &approve(&[".env", ".gitmodules"]));
    assert!(!report.has_problems(), "{report:?}");
    assert!(report.entries.iter().all(|e| e.approved));
    assert_eq!(fs::read_to_string(s.root.join(".env")).unwrap(), "A=1");
    assert_eq!(fs::read_to_string(s.root.join(".gitmodules")).unwrap(), "m");
}

#[test]
fn tm16_an_approval_for_a_path_not_in_the_plan_does_not_unlock_a_similar_one() {
    let s = sandbox();
    let report = apply_plan(
        &s.root,
        &[write_file(".github/workflows/ci.yml", "x")],
        &approve(&[".github/workflows", ".github/workflows/other.yml"]),
    );
    assert!(report.has_problems());
    assert_eq!(names(&s.root), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn tm16_a_symlinked_parent_into_a_denied_directory_needs_approval_of_the_real_path() {
    let s = sandbox();
    fs::create_dir_all(s.root.join(".github/workflows")).unwrap();
    std::os::unix::fs::symlink(".github/workflows", s.root.join("docs")).unwrap();
    let plan = [write_file("docs/ci.yml", "x")];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Refused(ApplyError::Sensitive {
            path: "docs/ci.yml".into(),
            needs: vec![".github/workflows/ci.yml".into()],
        })
    );
    let report = apply_plan(&s.root, &plan, &approve(&["docs/ci.yml"]));
    assert!(report.has_problems(), "the real path is not approved");
    let report = apply_plan(&s.root, &plan, &approve(&[".github/workflows/ci.yml"]));
    assert!(!report.has_problems());
    assert_eq!(
        fs::read_to_string(s.root.join(".github/workflows/ci.yml")).unwrap(),
        "x"
    );
}

#[test]
fn tm17_a_plan_marks_hook_and_mcp_entries_and_a_dry_run_writes_nothing() {
    let s = sandbox();
    let plan = [
        write_file("AGENTS.md", "# Rules\n"),
        write_file(
            ".claude/settings.json",
            r#"{"hooks": {"PostToolUse": [{"command": "slopvac"}]}}"#,
        ),
        write_file(
            ".cursor/mcp.json",
            r#"{"mcpServers": {"slopvac": {"command": "slopvac", "args": ["mcp"]}}}"#,
        ),
        write_file(
            ".opencode/opencode.json",
            r#"{"mcp": {"slopvac": {"type": "local"}}}"#,
        ),
        write_file("README.md", "# hooks and mcp servers are explained here\n"),
    ];
    let report = apply_plan(&s.root, &plan, &dry());
    let w = |i: usize| report.entries[i].warnings.clone();
    assert_eq!(w(0), [HarnessWarning::InstructionFile]);
    assert_eq!(w(1), [HarnessWarning::AgentDirectory, HarnessWarning::Hook]);
    assert_eq!(
        w(2),
        [HarnessWarning::AgentDirectory, HarnessWarning::McpServer]
    );
    assert_eq!(
        w(3),
        [HarnessWarning::AgentDirectory, HarnessWarning::McpServer]
    );
    assert_eq!(w(4), Vec::<HarnessWarning>::new());
    assert!(!report.wrote);
    assert_eq!(names(&s.root), Vec::<String>::new());
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm17_a_marked_plan_still_applies_when_the_caller_allows_writes() {
    let s = sandbox();
    let plan = [write_file(".claude/settings.json", r#"{"hooks": {}}"#)];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert!(report.entries[0].warnings.contains(&HarnessWarning::Hook));
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Done(WriteOutcome::Created)
    );
}

#[test]
fn tm18_windows_path_forms_and_reserved_names_are_refused() {
    let s = sandbox();
    for bad in [
        r"C:\Windows\x.md",
        "C:/x.md",
        r"docs\a.md",
        r"..\x.md",
        "CON",
        "docs/nul.txt",
        "Aux.md",
        "com1",
        "LPT9.log",
        "trailing-dot.",
        "trailing-space ",
        "docs/a.md:stream",
        ".git::$INDEX_ALLOCATION/x",
        "a<b.md",
        "a|b.md",
        "q?.md",
        "//server/share/x",
    ] {
        let report = apply_plan(&s.root, &[write_file(bad, "x")], &approve(&[bad]));
        assert!(
            matches!(
                only_status(&report, 0),
                EntryStatus::Refused(ApplyError::PathRefused(_))
            ),
            "{bad}: {:?}",
            only_status(&report, 0)
        );
    }
    assert_eq!(names(&s.root), Vec::<String>::new());
}

#[test]
fn tm18_paths_that_differ_only_in_case_collide_inside_a_plan() {
    let s = sandbox();
    let plan = [write_file("Docs/A.md", "1"), write_file("docs/a.MD", "2")];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(*only_status(&report, 0), ok_status("Docs/A.md"));
    assert_eq!(
        *only_status(&report, 1),
        EntryStatus::Refused(ApplyError::DuplicatePath("docs/a.MD".into()))
    );
    assert_eq!(names(&s.root), Vec::<String>::new());
}

#[test]
fn tm18_a_path_that_differs_only_in_case_from_an_existing_file_is_refused() {
    let s = sandbox();
    fs::write(s.root.join("Notes.md"), "mine").unwrap();
    let report = apply_plan(
        &s.root,
        &[write_file("notes.md", "x")],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Refused(ApplyError::CaseCollision {
            path: "notes.md".into(),
            existing: "Notes.md".into(),
        })
    );
    assert_eq!(names(&s.root), ["Notes.md"]);
    assert_eq!(fs::read_to_string(s.root.join("Notes.md")).unwrap(), "mine");
}

/// Name pairs that one case-insensitive file system treats as the same name,
/// worked out from Unicode `CaseFolding.txt` and the NTFS uppercase table.
#[rstest::rstest]
#[case::long_s("docs/\u{17F}.md", "docs/s.md")]
#[case::kelvin("docs/\u{212A}.md", "docs/k.md")]
#[case::dotless_i("docs/\u{131}.md", "docs/I.md")]
#[case::nfd_vs_nfc("docs/\u{E9}.md", "docs/e\u{301}.md")]
fn tm18_plan_entries_that_collide_under_full_folding_are_duplicates(
    #[case] first: &str,
    #[case] second: &str,
) {
    let s = sandbox();
    let plan = [write_file(first, "1"), write_file(second, "2")];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(*only_status(&report, 0), ok_status(first));
    assert_eq!(
        *only_status(&report, 1),
        EntryStatus::Refused(ApplyError::DuplicatePath(second.into()))
    );
}

#[rstest::rstest]
#[case::long_s("\u{17F}.md", "s.md")]
#[case::kelvin("\u{212A}.md", "k.md")]
#[case::dotless_i("\u{131}.md", "I.md")]
fn tm18_an_existing_file_that_differs_only_under_full_folding_is_a_case_twin(
    #[case] existing: &str,
    #[case] planned: &str,
) {
    let s = sandbox();
    fs::write(s.root.join(existing), "mine").unwrap();
    let report = apply_plan(
        &s.root,
        &[write_file(planned, "x")],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Refused(ApplyError::CaseCollision {
            path: planned.into(),
            existing: existing.into(),
        })
    );
}

/// The existing file has the decomposed name, as HFS+ stores it, and the plan
/// asks for the composed one.
#[test]
fn tm18_an_existing_decomposed_name_is_a_case_twin_of_the_composed_one() {
    let s = sandbox();
    fs::write(s.root.join("e\u{301}.md"), "mine").unwrap();
    let report = apply_plan(
        &s.root,
        &[write_file("\u{E9}.md", "x")],
        &PlanOptions::default(),
    );
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Refused(ApplyError::CaseCollision {
            path: "\u{E9}.md".into(),
            existing: "e\u{301}.md".into(),
        })
    );
}

/// Code points that ext4 casefold strips and HFS+ partly skips.
#[rstest::rstest]
#[case::zero_width_space("\u{200B}")]
#[case::soft_hyphen("\u{AD}")]
#[case::zero_width_non_joiner("\u{200C}")]
fn tm18_an_ignorable_code_point_does_not_hide_a_sensitive_path(#[case] ignorable: &str) {
    for clean in ["slopvac.toml", ".slopvac/plugins.lock"] {
        let at = clean.find('v').unwrap();
        let hidden = format!("{}{ignorable}{}", &clean[..at], &clean[at..]);
        let s = sandbox();
        let report = apply_plan(
            &s.root,
            &[write_file(&hidden, "x")],
            &PlanOptions::default(),
        );
        let status = only_status(&report, 0);
        assert!(
            matches!(status, EntryStatus::Refused(ApplyError::Sensitive { .. })),
            "{hidden:?}: {status:?}"
        );
        assert!(report.entries[0].sensitive.is_some(), "{hidden:?}");
        assert!(names(&s.root).is_empty(), "{hidden:?}");
    }
}

#[rstest::rstest]
#[case::zero_width_space("\u{200B}")]
#[case::soft_hyphen("\u{AD}")]
#[case::zero_width_non_joiner("\u{200C}")]
fn tm18_plan_entries_that_differ_only_by_an_ignorable_code_point_are_duplicates(
    #[case] ignorable: &str,
) {
    let s = sandbox();
    let second = format!("docs/no{ignorable}tes.md");
    let plan = [write_file("docs/notes.md", "1"), write_file(&second, "2")];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(*only_status(&report, 0), ok_status("docs/notes.md"));
    assert_eq!(
        *only_status(&report, 1),
        EntryStatus::Refused(ApplyError::DuplicatePath(second))
    );
}

// ---- review round 1 ----

#[cfg(unix)]
fn seam(
    s: &Sandbox,
    plan: &[RecipeFile],
    options: &PlanOptions,
    mut act: impl FnMut(usize, SeamStage),
) -> slopvac_plugins::apply::PlanReport {
    apply_plan_seam(&s.root, plan, options, false, &mut act)
}

#[test]
fn tm14_inserted_bytes_are_capped_at_the_limit_and_one_more_is_refused() {
    let at_cap = edit(0, 0, &"x".repeat(MAX_INSERTED_BYTES));
    assert_eq!(
        edited_bytes(b"", &[at_cap]).unwrap().len(),
        MAX_INSERTED_BYTES
    );
    let over = edit(0, 0, &"x".repeat(MAX_INSERTED_BYTES + 1));
    assert!(matches!(
        edited_bytes(b"", std::slice::from_ref(&over)),
        Err(ApplyError::InvalidEdits(_))
    ));
    let s = sandbox();
    fs::write(s.root.join("a.md"), "").unwrap();
    assert!(apply_edits(&s.root, "a.md", b"", std::slice::from_ref(&over)).is_err());
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "");
}

/// The cap counts the bytes of all edits together, not each edit alone.
#[test]
fn tm14_inserted_bytes_summed_over_several_edits_pass_the_cap_and_apply_nothing() {
    let half = MAX_INSERTED_BYTES / 2;
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab").unwrap();
    let over = [
        edit(0, 0, &"x".repeat(half)),
        edit(1, 1, &"y".repeat(half + 1)),
    ];
    assert!(matches!(
        apply_edits(&s.root, "a.md", b"ab", &over),
        Err(ApplyError::InvalidEdits(_))
    ));
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab");
    let at_cap = [edit(0, 0, &"x".repeat(half)), edit(1, 1, &"y".repeat(half))];
    assert_eq!(
        edited_bytes(b"ab", &at_cap).unwrap().len(),
        2 + MAX_INSERTED_BYTES
    );
}

/// The wire path: a fixer reply past either cap is `plugin-output-invalid`.
/// The finding spans 300,000 bytes, so the reply cap is 4 x 300,000 + 4,096,
/// held to the 1 MiB ceiling.
#[test]
fn tm14_a_fixer_reply_past_either_cap_is_plugin_output_invalid() {
    use serde_json::json;
    use slopvac_plugins::hooks::{Finding, MAX_EDITS_PER_DOCUMENT, parse_edits};

    let text = "a".repeat(300_000);
    let findings = [Finding {
        rule: "core:x".into(),
        severity: "warning".into(),
        span: Span {
            start: 0,
            end: 300_000,
        },
        message: "m".into(),
    }];
    let one = |t: &str| json!({"finding": 0, "span": {"start": 0, "end": 0}, "text": t});
    let code = |entry: Value| {
        parse_edits(&entry, &text, &findings)
            .unwrap_err()
            .slopvac_code()
            .to_owned()
    };
    let at_cap = json!({"edits": [one(&"y".repeat(MAX_INSERTED_BYTES))]});
    assert!(parse_edits(&at_cap, &text, &findings).is_ok());
    let over = json!({"edits": [one(&"y".repeat(MAX_INSERTED_BYTES + 1))]});
    assert_eq!(code(over), "plugin-output-invalid");
    let many: Vec<Value> = (0..=MAX_EDITS_PER_DOCUMENT).map(|_| one("")).collect();
    assert_eq!(code(json!({ "edits": many })), "plugin-output-invalid");
}

#[cfg(unix)]
#[test]
fn tm16_a_parent_swapped_for_a_symlink_after_the_check_cannot_redirect_the_write() {
    // Finding 1: the write must land in the directory that was checked.
    let s = sandbox();
    fs::create_dir_all(s.root.join(".github/workflows")).unwrap();
    fs::create_dir(s.root.join("docs")).unwrap();
    let plan = [write_file("docs/ci.yml", "x")];
    let report = seam(&s, &plan, &PlanOptions::default(), |_, stage| {
        if stage == SeamStage::Start {
            fs::rename(s.root.join("docs"), s.root.join("docs.orig")).unwrap();
            std::os::unix::fs::symlink(".github/workflows", s.root.join("docs")).unwrap();
        }
    });
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Done(WriteOutcome::Created)
    );
    assert_eq!(
        names(&s.root.join(".github/workflows")),
        Vec::<String>::new()
    );
    assert_eq!(
        fs::read_to_string(s.root.join("docs.orig/ci.yml")).unwrap(),
        "x"
    );
}

#[cfg(unix)]
#[test]
fn tm16_a_missing_parent_replaced_by_a_symlink_after_the_check_fails_and_writes_nothing() {
    let s = sandbox();
    fs::create_dir_all(s.root.join(".github/workflows")).unwrap();
    let plan = [write_file("new/ci.yml", "x")];
    let report = seam(&s, &plan, &PlanOptions::default(), |_, stage| {
        if stage == SeamStage::Start {
            std::os::unix::fs::symlink(".github/workflows", s.root.join("new")).unwrap();
        }
    });
    assert!(matches!(
        only_status(&report, 0),
        EntryStatus::Failed(ApplyError::Io(_))
    ));
    assert!(!report.wrote);
    assert_eq!(
        names(&s.root.join(".github/workflows")),
        Vec::<String>::new()
    );
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_a_hardlink_made_after_the_check_stops_a_plan_write() {
    let s = sandbox();
    fs::write(s.root.join("b.md"), "keep").unwrap();
    let plan = [write_file("b.md", "new")];
    let report = seam(&s, &plan, &PlanOptions::default(), |_, stage| {
        if stage == SeamStage::Start {
            fs::hard_link(s.root.join("b.md"), s.root.join("alias.md")).unwrap();
        }
    });
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Failed(ApplyError::Hardlinked("b.md".into()))
    );
    assert_eq!(fs::read_to_string(s.root.join("b.md")).unwrap(), "keep");
    assert_eq!(fs::read_to_string(s.root.join("alias.md")).unwrap(), "keep");
    assert_eq!(names(&s.root), ["alias.md", "b.md"]);
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn a_failure_part_way_reports_done_failed_and_not_attempted_exactly() {
    let s = sandbox();
    fs::write(s.root.join("b.md"), "keep").unwrap();
    let plan = [
        write_file("a.md", "first"),
        write_file("b.md", "second"),
        write_file("c.md", "third"),
    ];
    let report = seam(&s, &plan, &PlanOptions::default(), |i, stage| {
        if i == 1 && stage == SeamStage::Start {
            fs::write(s.root.join("b.md"), "changed by someone").unwrap();
        }
    });
    let statuses: Vec<_> = report.entries.iter().map(|e| e.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            EntryStatus::Done(WriteOutcome::Created),
            EntryStatus::Failed(ApplyError::SourceChanged("b.md".into())),
            EntryStatus::NotAttempted,
        ]
    );
    assert!(report.wrote);
    assert!(report.has_problems());
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "first");
    assert_eq!(
        fs::read_to_string(s.root.join("b.md")).unwrap(),
        "changed by someone"
    );
    assert!(!s.root.join("c.md").exists());
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_a_remove_of_an_absent_file_does_not_delete_one_created_after_the_check() {
    let s = sandbox();
    let plan = [file("late.md", RecipeAction::Remove, None)];
    let report = seam(&s, &plan, &PlanOptions::default(), |_, stage| {
        if stage == SeamStage::Start {
            fs::write(s.root.join("late.md"), "precious").unwrap();
        }
    });
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Failed(ApplyError::SourceChanged("late.md".into()))
    );
    assert_eq!(
        fs::read_to_string(s.root.join("late.md")).unwrap(),
        "precious"
    );
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn tm15_a_remove_does_not_delete_a_file_that_changed_after_the_check() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "checked").unwrap();
    let plan = [file("a.md", RecipeAction::Remove, None)];
    let report = seam(&s, &plan, &PlanOptions::default(), |_, stage| {
        if stage == SeamStage::BeforeCommit {
            fs::write(s.root.join("a.md"), "new work").unwrap();
        }
    });
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Failed(ApplyError::SourceChanged("a.md".into()))
    );
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "new work");
}

// Writes: Windows refuses these (see the native Windows tests below).
#[cfg(unix)]
#[test]
fn removing_a_file_in_a_missing_directory_is_absent_and_later_entries_still_apply() {
    let s = sandbox();
    let plan = [
        file("missing-dir/file.md", RecipeAction::Remove, None),
        write_file("after.md", "ok"),
    ];
    let report = apply_plan(&s.root, &plan, &PlanOptions::default());
    assert_eq!(
        *only_status(&report, 0),
        EntryStatus::Done(WriteOutcome::Absent)
    );
    assert_eq!(
        *only_status(&report, 1),
        EntryStatus::Done(WriteOutcome::Created)
    );
    assert_eq!(names(&s.root), ["after.md"]);
}

#[test]
fn tm18_windows_8_3_short_name_aliases_are_refused() {
    let s = sandbox();
    fs::create_dir_all(s.root.join(".github/workflows")).unwrap();
    for alias in [
        "GITHUB~1/WORKFL~1/ci.yml",
        ".github/WORKFL~1/ci.yml",
        "GIT~1/config",
        "PROGRA~1/x.md",
        "docs/NOTES~12.md",
    ] {
        let report = apply_plan_seam(
            &s.root,
            &[write_file(alias, "x")],
            &dry(),
            true,
            &mut |_, _| {},
        );
        assert!(
            matches!(
                only_status(&report, 0),
                EntryStatus::Refused(ApplyError::PathRefused(_))
            ),
            "{alias}: {:?}",
            only_status(&report, 0)
        );
    }
}

#[test]
fn tm18_names_with_a_tilde_but_no_digits_are_not_taken_for_aliases() {
    let s = sandbox();
    let report = apply_plan_seam(
        &s.root,
        &[write_file("docs/a~b.md", "x")],
        &dry(),
        true,
        &mut |_, _| {},
    );
    // Only the Windows write refusal remains.
    assert!(matches!(
        only_status(&report, 0),
        EntryStatus::Refused(ApplyError::UnsupportedOnWindows(_))
    ));
}

#[test]
fn h22_on_windows_every_mutating_entry_is_refused_and_nothing_is_written() {
    let s = sandbox();
    fs::write(s.root.join("old.md"), "old").unwrap();
    let plan = [
        write_file("new.md", "x"),
        file("old.md", RecipeAction::Remove, None),
        file("m.md", RecipeAction::Merge, Some("x")),
        write_file("dir/deep.md", "x"),
    ];
    for dry_run in [false, true] {
        let options = PlanOptions {
            dry_run,
            allow_sensitive: vec![],
        };
        let report = apply_plan_seam(&s.root, &plan, &options, true, &mut |_, _| {
            panic!("nothing may be applied on Windows")
        });
        for (i, e) in report.entries.iter().enumerate() {
            assert!(
                matches!(
                    e.status,
                    EntryStatus::Refused(ApplyError::UnsupportedOnWindows(_))
                ),
                "{i}: {:?}",
                e.status
            );
        }
        assert!(!report.wrote);
    }
    assert_eq!(names(&s.root), ["old.md"]);
    assert_eq!(fs::read_to_string(s.root.join("old.md")).unwrap(), "old");
}

#[cfg(unix)]
#[test]
fn tm17_the_resolved_destination_is_marked_as_well_as_the_written_path() {
    let s = sandbox();
    fs::create_dir(s.root.join(".claude")).unwrap();
    std::os::unix::fs::symlink(".claude", s.root.join("notes")).unwrap();
    let plan = [write_file("notes/settings.json", r#"{"hooks": {}}"#)];
    let report = apply_plan(&s.root, &plan, &dry());
    assert_eq!(
        report.entries[0].warnings,
        [HarnessWarning::AgentDirectory, HarnessWarning::Hook]
    );
    assert_eq!(
        report.entries[0].resolved.as_deref(),
        Some(".claude/settings.json")
    );
}

// ---- review round 2 ----

#[test]
fn h22_edits_are_validated_first_and_then_refused_on_windows() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab").unwrap();
    let valid = apply_edits_seam(&s.root, "a.md", b"ab", &[edit(0, 1, "X")], true);
    assert_eq!(valid, Err(ApplyError::UnsupportedOnWindows("a.md".into())));
    let invalid = apply_edits_seam(&s.root, "a.md", b"ab", &[edit(0, 9, "X")], true);
    assert!(matches!(invalid, Err(ApplyError::InvalidEdits(_))));
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab");
    assert_eq!(names(&s.root), ["a.md"]);
}

#[test]
fn the_preview_of_edits_still_works_where_writing_is_refused() {
    assert_eq!(edited_bytes(b"ab", &[edit(0, 1, "X")]).unwrap(), b"Xb");
}

#[cfg(windows)]
#[test]
fn native_windows_apply_edits_refuses_and_leaves_the_file_alone() {
    let s = sandbox();
    fs::write(s.root.join("a.md"), "ab").unwrap();
    let got = apply_edits(&s.root, "a.md", b"ab", &[edit(0, 1, "X")]);
    assert_eq!(got, Err(ApplyError::UnsupportedOnWindows("a.md".into())));
    assert_eq!(fs::read_to_string(s.root.join("a.md")).unwrap(), "ab");
    assert_eq!(names(&s.root), ["a.md"]);
}

#[cfg(windows)]
#[test]
fn native_windows_plan_refuses_every_mutating_entry_in_a_dry_run_and_a_real_run() {
    let s = sandbox();
    fs::write(s.root.join("old.md"), "old").unwrap();
    let plan = [
        write_file("new.md", "x"),
        file("old.md", RecipeAction::Remove, None),
    ];
    for options in [PlanOptions::default(), dry()] {
        let report = apply_plan(&s.root, &plan, &options);
        assert_eq!(
            *only_status(&report, 0),
            EntryStatus::Refused(ApplyError::UnsupportedOnWindows("new.md".into()))
        );
        assert_eq!(
            *only_status(&report, 1),
            EntryStatus::Refused(ApplyError::UnsupportedOnWindows("old.md".into()))
        );
        assert!(!report.wrote);
    }
    assert_eq!(names(&s.root), ["old.md"]);
}
