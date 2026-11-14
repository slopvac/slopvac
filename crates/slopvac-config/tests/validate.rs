//! `slopvac validate` through its command definition.

use std::fs;
use std::path::PathBuf;

use slopvac_config::validate::{FRONTMATTER_SCHEMA, command, run};
use slopvac_config::{Code, NoPresets};
use tempfile::TempDir;

struct Project {
    dir: TempDir,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        fs::canonicalize(self.dir.path()).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.root().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn validate(&self, args: &[&str]) -> (u8, String) {
        let mut line = vec!["validate"];
        line.extend_from_slice(args);
        let matches = command().try_get_matches_from(line).unwrap();
        let mut out = Vec::new();
        let code = run(&matches, &self.root(), &NoPresets, &mut out).unwrap();
        (code, String::from_utf8(out).unwrap())
    }
}

#[test]
fn a_clean_project_exits_zero() {
    let p = Project::new();
    p.write("slopvac.toml", "schema_version = 3\n");
    p.write(
        "docs/a.md",
        "---\nslopvac:\n  v: 3\n  locale: en-GB\n---\nBody\n",
    );
    let (code, out) = p.validate(&["docs"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "checked 2 file(s): 0 problem(s)\n")
    );
}

#[test]
fn a_front_matter_error_is_rendered_with_its_position_and_exits_one() {
    let p = Project::new();
    p.write("slopvac.toml", "schema_version = 3\n");
    p.write(
        "docs/a.md",
        "---\nslopvac:\n  v: 3\n  profil: strict\n---\nBody\n",
    );
    let (code, out) = p.validate(&["docs"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("error[config-unknown-key]: unknown key `profil`"),
        "{out}"
    );
    assert!(out.contains("--> docs/a.md:4:3"), "{out}");
    assert!(out.contains("did you mean `profile`?"), "{out}");
    assert!(out.ends_with("checked 2 file(s): 1 problem(s)\n"), "{out}");
}

#[test]
fn each_error_class_in_a_document_gets_its_own_code() {
    let cases: [(&str, &str, Code); 5] = [
        ("a.md", "---\nslopvac: [\n---\n", Code::Parse),
        (
            "a.md",
            "---\nslopvac:\n  locale: en-GB\n---\n",
            Code::MissingKey,
        ),
        (
            "a.md",
            "---\nslopvac:\n  v: 3\n  locale: fr\n---\n",
            Code::InvalidValue,
        ),
        (
            "a.md",
            "---\nslopvac:\n  v: 3\n  rules:\n    core:x: off\n---\n",
            Code::FrontmatterFieldDenied,
        ),
        (
            "a.md",
            "<!-- slopvac: contexts=code -->\n",
            Code::UnknownKey,
        ),
    ];
    for (file, text, code) in cases {
        let p = Project::new();
        p.write("slopvac.toml", "schema_version = 3\n");
        p.write(file, text);
        let (exit, out) = p.validate(&[file]);
        assert_eq!(exit, 1, "{text:?}");
        assert!(
            out.contains(&format!("[{}]", code.as_str())),
            "{text:?}: {out}"
        );
    }
}

#[test]
fn a_2x_config_exits_two_and_points_at_migrate() {
    let p = Project::new();
    p.write("slopvac.toml", "profile = \"normal\"\n");
    p.write("a.md", "text\n");
    let (code, out) = p.validate(&["a.md"]);
    assert_eq!(code, 2);
    assert!(out.contains("config-legacy-version"), "{out}");
    assert!(out.contains("slopvac migrate"), "{out}");
}

#[test]
fn an_invalid_config_exits_two_with_the_key_position() {
    let p = Project::new();
    p.write(
        "slopvac.toml",
        "schema_version = 3\n\n[thresholds]\nmax_eror = 0\n",
    );
    let (code, out) = p.validate(&[]);
    assert_eq!(code, 2);
    assert!(out.contains("--> slopvac.toml:4:1"), "{out}");
}

#[test]
fn a_terms_file_that_does_not_exist_is_reported_at_the_setting() {
    let p = Project::new();
    p.write(
        "slopvac.toml",
        "schema_version = 3\n\n[terms]\nfiles = [\"terms.toml\", \"present.toml\"]\n",
    );
    p.write("present.toml", "");
    let (code, out) = p.validate(&[]);
    assert_eq!(code, 1);
    assert!(
        out.contains("`terms.files` names `terms.toml`, which is not a file"),
        "{out}"
    );
    assert!(out.contains("--> slopvac.toml:4:1"), "{out}");
    assert!(!out.contains("present.toml"), "{out}");
}

#[test]
fn excluded_documents_are_skipped() {
    let p = Project::new();
    p.write(
        "slopvac.toml",
        "schema_version = 3\nexclude = [\"vendor/**\"]\n",
    );
    p.write("vendor/a.md", "---\nslopvac:\n  nonsense: 1\n---\n");
    p.write("docs/b.md", "text\n");
    let (code, out) = p.validate(&["."]);
    assert_eq!(
        (code, out.as_str()),
        (0, "checked 2 file(s): 0 problem(s)\n")
    );
}

#[test]
fn walking_a_directory_reads_documents_only() {
    let p = Project::new();
    p.write("slopvac.toml", "schema_version = 3\n");
    p.write("docs/a.md", "text\n");
    p.write("docs/b.rst", "text\n");
    p.write("docs/image.png", "not text");
    p.write("docs/code.rs", "// slopvac: nonsense=1\n");
    let (code, out) = p.validate(&["docs"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "checked 3 file(s): 0 problem(s)\n")
    );
}

#[test]
fn an_explicit_config_applies_to_every_file() {
    let p = Project::new();
    p.write(
        "custom/team.toml",
        "schema_version = 3\n[frontmatter]\nallow = [\"rules\"]\n",
    );
    p.write(
        "a.md",
        "---\nslopvac:\n  v: 3\n  rules:\n    core:x: \"off\"\n---\n",
    );
    let (code, _) = p.validate(&["a.md"]);
    assert_eq!(code, 1);
    let (code, out) = p.validate(&["--config", "custom/team.toml", "a.md"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "checked 2 file(s): 0 problem(s)\n")
    );
}

#[test]
fn quiet_prints_problems_only() {
    let p = Project::new();
    p.write("slopvac.toml", "schema_version = 3\n");
    let (code, out) = p.validate(&["--quiet"]);
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn the_schema_flag_prints_the_front_matter_schema_for_editors() {
    let p = Project::new();
    let (code, out) = p.validate(&["--schema", "frontmatter"]);
    assert_eq!(code, 0);
    assert_eq!(out, FRONTMATTER_SCHEMA);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        json["$id"]
            .as_str()
            .unwrap()
            .ends_with("/frontmatter.schema.json")
    );
}
