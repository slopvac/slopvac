//! Safety and fidelity tests for `slopvac setup`: containment, scale, line endings.

use std::fs;
use std::path::Path;

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

#[cfg(unix)]
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
    fn symlinks_into_dot_git_are_refused_for_files_and_ancestors() {
        let cases = [
            ("CLAUDE.md", ".git/config"),
            ("CLAUDE.md", ".git/info/x.md"),
            ("CLAUDE.md", "docs/../.git/y.md"),
            ("CLAUDE.md", ".Git/info/x.md"),
            ("CLAUDE.md", ".GIT/y.md"),
            ("CLAUDE.md", "docs/../.gIt/y.md"),
            (".claude", ".git"),
            (".claude", ".Git"),
            (".claude", ".GIT"),
            (".claude", "sub/../.gIT"),
            (".claude", "sub/../.git"),
        ];
        for (link, to) in cases {
            let project = tempfile::tempdir().unwrap();
            let git = project.path().join(".git");
            fs::create_dir_all(git.join("info")).unwrap();
            fs::create_dir(project.path().join("docs")).unwrap();
            fs::create_dir(project.path().join("sub")).unwrap();
            fs::write(git.join("config"), "[core]\n").unwrap();
            symlink(to, project.path().join(link)).unwrap();
            for (check, remove) in [(false, false), (true, false), (false, true)] {
                let o = Options {
                    check,
                    remove,
                    ..opts(project.path(), "claude")
                };
                match execute(&o) {
                    Err(e) => assert_eq!(e.code(), "steering-unsafe-target", "{link}->{to}"),
                    Ok(out) => assert_ne!(out.exit, 0, "{link}->{to} {check} {remove}"),
                }
            }
            assert_eq!(fs::read_to_string(git.join("config")).unwrap(), "[core]\n");
            assert!(!git.join("info/x.md").exists() && !git.join("y.md").exists());
            assert_eq!(fs::read_dir(&git).unwrap().count(), 2, "{link}->{to}");
        }
    }

    #[test]
    fn a_symlink_to_a_non_markdown_file_is_refused() {
        let project = tempfile::tempdir().unwrap();
        fs::write(project.path().join("notes.txt"), "keep\n").unwrap();
        symlink("notes.txt", project.path().join("CLAUDE.md")).unwrap();
        let err = execute(&opts(project.path(), "claude")).unwrap_err();
        assert_eq!(err.code(), "steering-unsafe-target");
        assert_eq!(
            fs::read_to_string(project.path().join("notes.txt")).unwrap(),
            "keep\n"
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
        "# A",
        "# A\n\n\n",
        "# A\r\n\r\n\r\n",
        "# A\r\r\r",
        "# A\n  \n",
        "\n",
        "\n\n",
        "# A\n```\ncode\n```",
        "a\nb\r",
        "a\r\nb\n",
        "a\nb\r\n",
        "a\rb\n",
        "a\nb\rc",
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
fn a_file_without_a_final_newline_keeps_none_after_remove() {
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
    assert_eq!(fs::read_to_string(&file).unwrap(), "# A");
}

#[test]
fn a_lone_cr_after_lf_lines_does_not_merge_into_crlf() {
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("CLAUDE.md");
    fs::write(&file, "a\nb\r").unwrap();
    let o = Options {
        agent: false,
        no_skill: true,
        ..opts(project.path(), "claude")
    };
    execute(&o).unwrap();
    let installed = fs::read(&file).unwrap();
    assert!(installed.starts_with(b"a\nb\r\r<!-- slopvac:begin -->"));
    execute(&Options { remove: true, ..o }).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"a\nb\r");
}
