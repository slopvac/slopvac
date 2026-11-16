//! The generated built-in terms reference page.
#![cfg(test)]

use std::path::{Path, PathBuf};
use std::process::Command;

use slopvac_terms::{TermsFile, reference_pages};

fn page() -> String {
    let pages = reference_pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].0, "terms/index.md");
    pages[0].1.clone()
}

#[test]
fn every_builtin_term_appears_with_its_variants_and_mode() {
    let text = page();
    for t in TermsFile::builtin().terms {
        let escaped = t.term.replace('|', "\\|");
        assert!(text.contains(&format!("| `{escaped}` | ")), "{}", t.term);
        for v in &t.variants {
            assert!(text.contains(&format!("`{v}`")), "{}: {v}", t.term);
        }
    }
}

#[test]
fn the_page_states_the_count_provenance_and_modes() {
    let text = page();
    let n = TermsFile::builtin().terms.len();
    assert!(
        text.starts_with(&format!("# Built-in terms\n\nslopvac ships {n} terms")),
        "{text}"
    );
    assert!(text.contains("Apache-2.0"));
    assert!(text.contains("**exact spelling only**"));
}

#[test]
fn rows_show_case_modes_and_group_by_kind() {
    let text = page();
    assert!(
        text.contains("| `Python` | - | exact spelling only |"),
        "{text}"
    );
    assert!(
        text.contains("| `GitHub` | - | flags wrong case |"),
        "{text}"
    );
    assert!(
        text.contains("| `Kubernetes` | `K8s` | flags wrong case |"),
        "{text}"
    );
    assert!(text.contains("\n## protocol\n\n| Term |"), "{text}");
}

/// The cargo profile this test binary was built with, from its path:
/// `target/<dir>/deps/<test>`, where `debug` is the `dev` profile.
fn profile() -> String {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().file_name().unwrap();
    match dir.to_str().unwrap() {
        "debug" => "dev".to_owned(),
        other => other.to_owned(),
    }
}

/// Build the example with the test's own profile and return the executable
/// path that cargo reports, so any profile or target directory works. Building
/// here also covers `cargo test --test reference`, which skips examples.
fn example() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let output = Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--example", "reference"])
                .args(["--profile", &profile(), "--message-format=json"])
                .args([
                    "--manifest-path",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "building the example failed");
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|m| {
                    m["reason"] == "compiler-artifact" && m["target"]["name"] == "reference"
                })
                .find_map(|m| m["executable"].as_str().map(PathBuf::from))
                .expect("cargo reported no executable for the example")
        })
        .clone()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-terms-ref-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_example_writes_terms_index_md_under_the_out_dir() {
    let out = scratch("write");
    let status = Command::new(example()).arg(&out).status().unwrap();
    assert!(status.success());
    let written = std::fs::read_to_string(out.join("terms/index.md")).unwrap();
    assert_eq!(written, page());
    // Running again over existing output succeeds and rewrites the page.
    assert!(
        Command::new(example())
            .arg(&out)
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn the_example_fails_without_writing_when_the_out_dir_is_a_file() {
    let dir = scratch("io-failure");
    let blocker: &Path = &dir.join("not-a-dir");
    std::fs::write(blocker, "x").unwrap();
    let output = Command::new(example()).arg(blocker).output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
    assert_eq!(std::fs::read_to_string(blocker).unwrap(), "x");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_example_needs_an_out_dir_argument() {
    let output = Command::new(example()).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: reference <out-dir>"));
}
