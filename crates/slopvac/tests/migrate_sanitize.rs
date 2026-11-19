//! The installed binary neutralises escape sequences in migrate's error text.

use assert_cmd::Command;
use std::fs;

#[test]
fn stderr_carries_no_raw_escape_from_a_file_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("mkdir");
    fs::write(
        dir.path().join("docs/\u{1b}[31mred.md"),
        "<!-- slopvac-allow: rule=no.such-rule reason=x -->\n",
    )
    .expect("write");
    let out = Command::cargo_bin("slopvac")
        .expect("binary builds")
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(2)
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).expect("utf8");
    assert!(!err.contains('\u{1b}'), "{err:?}");
    assert!(err.contains("error[config-legacy-dropped]: "), "{err:?}");
    assert!(err.contains("docs/\\u{1b}[31mred.md:1:"), "{err:?}");
}
