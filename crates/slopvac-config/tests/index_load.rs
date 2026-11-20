//! Extra-index sources never reach rendered output, and the extra-index count is capped.

mod common;

use std::fs;
use std::path::PathBuf;

use common::NoPresets;
use rstest::rstest;
use slopvac_config::tree::parse_toml;
use slopvac_config::validate::{command, run};
use slopvac_config::{
    BudgetData, Code, ConfigBudget, ConfigError, MAX_INDEX_EXTRA, SourceFile, load, render,
    validate_config,
};
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

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let p = self.root().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
        p
    }

    fn validate(&self) -> (u8, String) {
        let matches = command().try_get_matches_from(["validate"]).unwrap();
        let mut out = Vec::new();
        let code = run(&matches, &self.root(), &NoPresets, &mut out).unwrap();
        (code, String::from_utf8(out).unwrap())
    }
}

fn entries(count: usize) -> String {
    (0..count)
        .map(|i| {
            format!("[[index.extra]]\nname = \"i{i:02}\"\nsource = \"https://i{i:02}.example/index.json\"\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn budget(err: ConfigError) -> BudgetData {
    let ConfigError::Rejected(r) = err else {
        panic!("expected a rejection, got {err}");
    };
    assert_eq!(r.diagnostics[0].code, Code::BudgetExceeded);
    *r.diagnostics
        .into_iter()
        .next()
        .and_then(|d| d.data)
        .expect("a budget diagnostic carries data")
}

#[rstest]
#[case::user_info("\"https://user:SECRET@plugins.acme.example/index.json\"")]
#[case::literal_string("'https://user:SECRET@plugins.acme.example/index.json'")]
#[case::unsupported_scheme("\"ftp://plugins.acme.example/index.json?token=SECRET\"")]
fn a_refused_source_is_masked_in_the_validate_output(#[case] literal: &str) {
    let p = Project::new();
    p.write(
        "slopvac.toml",
        &format!("schema_version = 3\n\n[[index.extra]]\nname = \"acme\"\nsource = {literal}\n"),
    );
    let (code, out) = p.validate();
    assert_eq!(code, 2, "{out}");
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("`index.extra[0].source` is refused"), "{out}");
    assert!(out.contains("--> slopvac.toml:5:10"), "{out}");
    assert!(out.contains("source = "), "{out}");
}

#[test]
fn a_valid_source_with_a_token_is_masked_when_another_key_is_wrong() {
    let p = Project::new();
    p.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[index.extra]]\nname = \"slopvac\"\nsource = \"https://plugins.acme.example/index.json?token=SECRET\"\n",
    );
    let (code, out) = p.validate();
    assert_eq!(code, 2, "{out}");
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("--> slopvac.toml:4:8"), "{out}");
}

#[test]
fn the_public_render_masks_every_source_and_keeps_the_text_around_it() {
    let text = "schema_version = 3\n\n[[index.extra]]\nname = \"a\"\nsource = \"https://u:SECRET@h.example/x\" # note\n\n[[index.extra]]\nname = \"a\"\nsource = \"https://h.example/?token=SECRET\"\n";
    let diags = validate_config(&parse_toml(text).unwrap(), text);
    assert_ne!(diags, Vec::new());
    let out = render(
        SourceFile {
            name: "slopvac.toml",
            text,
        },
        &diags,
    );
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("# note"), "{out}");
    assert!(out.contains("source = \""), "{out}");
}

#[test]
fn a_list_of_sixteen_extra_indexes_loads() {
    let p = Project::new();
    let path = p.write(
        "slopvac.toml",
        &format!("schema_version = 3\n\n{}", entries(MAX_INDEX_EXTRA)),
    );
    assert!(load(&path, &NoPresets).is_ok());
}

#[test]
fn a_list_of_seventeen_extra_indexes_is_refused_with_its_budget() {
    let p = Project::new();
    let path = p.write(
        "slopvac.toml",
        &format!("schema_version = 3\n\n{}", entries(MAX_INDEX_EXTRA + 1)),
    );
    let data = budget(load(&path, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::IndexExtra);
    assert_eq!(data.budget.as_str(), "index-extra");
    assert_eq!(data.limit, 16);
    assert_eq!(data.path, "slopvac.toml");
}

#[test]
fn the_cap_comes_before_the_duplicate_name_check() {
    let p = Project::new();
    let one = "[[index.extra]]\nname = \"same\"\nsource = \"https://a.example/i.json\"\n\n";
    let path = p.write(
        "slopvac.toml",
        &format!("schema_version = 3\n\n{}", one.repeat(MAX_INDEX_EXTRA + 1)),
    );
    let data = budget(load(&path, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::IndexExtra);
}

#[test]
fn the_schema_check_refuses_seventeen_extra_indexes_at_the_list() {
    let text = format!("schema_version = 3\n\n{}", entries(MAX_INDEX_EXTRA + 1));
    let diags = validate_config(&parse_toml(&text).unwrap(), &text);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].pointer(), "/index/extra");
}

#[test]
fn an_extended_file_with_seventeen_extra_indexes_is_refused_and_sixteen_plus_sixteen_load() {
    let p = Project::new();
    p.write(
        "ok.toml",
        &format!("schema_version = 3\n\n{}", entries(MAX_INDEX_EXTRA)),
    );
    let project = p.write(
        "slopvac.toml",
        &format!(
            "schema_version = 3\nextends = [\"ok.toml\"]\n\n{}",
            entries(MAX_INDEX_EXTRA)
        ),
    );
    assert!(load(&project, &NoPresets).is_ok());

    p.write(
        "big.toml",
        &format!("schema_version = 3\n\n{}", entries(MAX_INDEX_EXTRA + 1)),
    );
    let project = p.write(
        "other.toml",
        "schema_version = 3\nextends = [\"big.toml\"]\n",
    );
    let data = budget(load(&project, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::IndexExtra);
    assert_eq!(data.path, "big.toml");
}

#[rstest]
#[case::escaped_key("\"sour\\u0063e\" = \"https://user:SECRET@h.example/x?token=SECRET\"")]
#[case::quoted_key("'source' = \"https://user:SECRET@h.example/x?token=SECRET\"")]
#[case::array_value("source = [\"https://user:SECRET@h.example/x?token=SECRET\"]")]
#[case::inline_table_value("source = { url = \"https://user:SECRET@h.example/?token=SECRET\" }")]
fn every_source_spelling_and_value_type_is_masked_in_output(#[case] line: &str) {
    let p = Project::new();
    let text = format!("schema_version = 3\n\n[[index.extra]]\nname = \"acme\"\n{line}\n");
    p.write("slopvac.toml", &text);
    let (code, out) = p.validate();
    assert_eq!(code, 2, "{out}");
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("slopvac.toml:5:"), "{out}");

    let diags = validate_config(&parse_toml(&text).unwrap(), &text);
    let rendered = render(
        SourceFile {
            name: "slopvac.toml",
            text: &text,
        },
        &diags,
    );
    assert!(!rendered.contains("SECRET"), "{rendered}");
}

#[test]
fn a_config_that_does_not_parse_shows_its_location_and_no_excerpt() {
    let p = Project::new();
    let text = "schema_version = 3\n\n[[index.extra]]\nname = \"acme\"\nsource = \"https://user:SECRET@h.example/x?token=SECRET\"\nbroken = = 1\n";
    let path = p.write("slopvac.toml", text);

    let (code, out) = p.validate();
    assert_eq!(code, 2, "{out}");
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("slopvac.toml:6:"), "{out}");
    assert!(out.contains("not valid TOML"), "{out}");

    let ConfigError::Rejected(r) = load(&path, &NoPresets).unwrap_err() else {
        panic!("expected a rejection");
    };
    let rendered = r.render();
    assert!(!rendered.contains("SECRET"), "{rendered}");
    assert!(rendered.contains("slopvac.toml:6:"), "{rendered}");

    let at = text.find("broken").unwrap();
    let d = slopvac_config::Diagnostic {
        code: Code::InvalidValue,
        message: "bad".into(),
        label: "here".into(),
        span: at..at + 6,
        help: None,
        path: vec![],
        data: None,
    };
    let public = render(
        SourceFile {
            name: "slopvac.toml",
            text,
        },
        &[d],
    );
    assert!(!public.contains("SECRET"), "{public}");
    assert!(public.contains("slopvac.toml:6:1"), "{public}");
}
