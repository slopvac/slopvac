//! Safety and fidelity tests for `slopvac setup`: containment, scale, line endings.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use slopvac_agents::content::BlockKind;
use slopvac_agents::setup::{Options, execute};

fn opts(root: &Path, harness: &str) -> Options {
    Options {
        harness: Some(harness.into()),
        list: false,
        remove: false,
        check: false,
        root: root.into(),
        block: BlockKind::Standard,
        no_skill: false,
        agent: true,
        reviewer_model: None,
    }
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).unwrap().next().is_none()
}

#[cfg(unix)]
mod containment {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Each harness directory that setup writes through, as a symlink to outside the project.
    const DIRS: [(&str, &str); 4] = [
        ("claude", ".claude"),
        ("opencode", ".agents"),
        ("opencode", ".opencode"),
        ("kiro", ".kiro"),
    ];

    #[test]
    fn an_ancestor_symlinked_outside_is_refused_by_install_check_and_remove() {
        for (harness, link) in DIRS {
            let project = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            symlink(outside.path(), project.path().join(link)).unwrap();
            for (check, remove) in [(false, false), (true, false), (false, true)] {
                let o = Options {
                    check,
                    remove,
                    agent: harness != "kiro",
                    ..opts(project.path(), harness)
                };
                match execute(&o) {
                    Err(e) => assert_eq!(e.code(), "steering-unsafe-target", "{harness} {link}"),
                    Ok(out) => assert_ne!(out.exit, 0, "{harness} {link} {check} {remove}"),
                }
                assert!(is_empty(outside.path()), "{harness} {link} wrote outside");
            }
        }
    }

    #[test]
    fn a_swapped_ancestor_cannot_redirect_a_later_run() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        execute(&opts(project.path(), "claude")).unwrap();
        fs::rename(
            project.path().join(".claude"),
            project.path().join(".claude-real"),
        )
        .unwrap();
        symlink(outside.path(), project.path().join(".claude")).unwrap();
        let err = execute(&Options {
            remove: true,
            ..opts(project.path(), "claude")
        })
        .unwrap_err();
        assert_eq!(err.code(), "steering-unsafe-target");
        assert!(is_empty(outside.path()));
        assert!(
            project
                .path()
                .join(".claude-real/skills/slopvac-prose/SKILL.md")
                .exists()
        );
    }

    #[test]
    fn steering_file_symlinked_outside_is_refused_absolute_and_relative() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("elsewhere.md");
        fs::write(&target, "keep\n").unwrap();
        for link in [target.clone(), Path::new("../elsewhere.md").to_path_buf()] {
            let project = tempfile::tempdir_in(outside.path()).unwrap();
            symlink(&link, project.path().join("CLAUDE.md")).unwrap();
            let err = execute(&opts(project.path(), "claude")).unwrap_err();
            assert_eq!(err.code(), "steering-unsafe-target", "{link:?}");
            assert_eq!(fs::read_to_string(&target).unwrap(), "keep\n");
        }
    }

    #[test]
    fn symlinks_that_stay_inside_the_project_work_and_survive() {
        let project = tempfile::tempdir().unwrap();
        fs::write(project.path().join("AGENTS.md"), "# Mine\n").unwrap();
        symlink("AGENTS.md", project.path().join("CLAUDE.md")).unwrap();
        fs::create_dir(project.path().join("shared")).unwrap();
        symlink("shared", project.path().join(".agents")).unwrap();
        execute(&Options {
            agent: false,
            ..opts(project.path(), "claude")
        })
        .unwrap();
        assert!(
            fs::symlink_metadata(project.path().join("CLAUDE.md"))
                .unwrap()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(project.path().join("AGENTS.md"))
                .unwrap()
                .contains("slopvac:begin")
        );
        execute(&Options {
            agent: false,
            ..opts(project.path(), "opencode")
        })
        .unwrap();
        assert!(
            project
                .path()
                .join("shared/skills/slopvac-prose/SKILL.md")
                .exists()
        );
    }

    #[test]
    fn a_symlink_loop_is_refused() {
        let project = tempfile::tempdir().unwrap();
        symlink("CLAUDE.md", project.path().join("CLAUDE.md")).unwrap();
        let err = execute(&opts(project.path(), "claude")).unwrap_err();
        assert_eq!(err.code(), "steering-unsafe-target");
    }
}

#[test]
fn marker_scan_stays_linear_on_many_code_blocks() {
    let project = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for i in 0..20_000 {
        write!(text, "```\ncode {i}\n```\n\nline {i}\n\n").unwrap();
    }
    for i in 0..60_000 {
        writeln!(text, "filler {i}").unwrap();
    }
    fs::write(project.path().join("CLAUDE.md"), &text).unwrap();
    let start = Instant::now();
    execute(&opts(project.path(), "claude")).unwrap();
    execute(&Options {
        remove: true,
        ..opts(project.path(), "claude")
    })
    .unwrap();
    let took = start.elapsed();
    assert!(took < Duration::from_secs(5), "scan took {took:?}");
    assert_eq!(
        fs::read_to_string(project.path().join("CLAUDE.md")).unwrap(),
        text
    );
}

#[test]
fn remove_restores_the_original_bytes_for_every_line_ending() {
    let cases = [
        "",
        "# A\n",
        "# A\n\nText\n",
        "# A\r\n\r\nText\r\n",
        "# A\r\rText\r",
        "\u{feff}# A\n",
        "```md\n<!-- slopvac:begin -->\n```\n",
        "# A\n\n<!-- x -->\n\nMore\n",
    ];
    for original in cases {
        let project = tempfile::tempdir().unwrap();
        let file = project.path().join("CLAUDE.md");
        fs::write(&file, original).unwrap();
        let o = Options {
            agent: false,
            no_skill: true,
            ..opts(project.path(), "claude")
        };
        execute(&o).unwrap();
        let installed = fs::read_to_string(&file).unwrap();
        assert!(installed.contains("slopvac:begin"), "{original:?}");
        execute(&o).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            installed,
            "idempotent {original:?}"
        );
        execute(&Options { remove: true, ..o }).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), original, "{original:?}");
    }
}

#[test]
fn the_block_uses_the_file_line_ending() {
    let cases = [("# A\r\n", "\r\n"), ("# A\r", "\r"), ("# A\n", "\n")];
    for (original, nl) in cases {
        let project = tempfile::tempdir().unwrap();
        let file = project.path().join("CLAUDE.md");
        fs::write(&file, original).unwrap();
        execute(&Options {
            agent: false,
            no_skill: true,
            ..opts(project.path(), "claude")
        })
        .unwrap();
        let text = fs::read_to_string(&file).unwrap();
        let stripped = text.replace(nl, "");
        assert!(
            !stripped.contains(['\r', '\n']),
            "{original:?}: mixed line endings"
        );
    }
}

#[test]
fn a_file_without_a_final_newline_gains_exactly_one_on_remove() {
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("CLAUDE.md");
    fs::write(&file, "# A").unwrap();
    let o = Options {
        agent: false,
        no_skill: true,
        ..opts(project.path(), "claude")
    };
    execute(&o).unwrap();
    execute(&Options { remove: true, ..o }).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "# A\n");
}
