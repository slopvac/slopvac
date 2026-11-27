//! `slopvac lint --explain-config --format json` prints the resolved settings of each input as
//! one JSON document (`schemas/v3/explain-config.schema.json`, `docs/contract/config.md`
//! section 1.4.1).
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};
use snapbox::cmd::{Command, cargo_bin};

/// A project locale, and a `[[overrides]]` block that gives `docs/**` another one.
const CONFIG: &str = "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n";

/// A temp project with `slopvac.toml` set to `config`, `a.md` and `docs/a.md`.
fn project_with(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("write config");
    fs::write(dir.path().join("a.md"), "# Title\n\nText.\n").expect("write document");
    fs::create_dir(dir.path().join("docs")).expect("docs dir");
    fs::write(dir.path().join("docs/a.md"), "# Title\n\nText.\n").expect("write document");
    dir
}

fn project() -> tempfile::TempDir {
    project_with(CONFIG)
}

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
}

/// The document `--explain-config --format json` prints for `args`, after an exit 0 and an
/// empty stderr.
fn explained(dir: &Path, args: &[&str]) -> Value {
    let assert = slopvac(dir)
        .args(["lint", "--explain-config", "--format", "json"])
        .args(args)
        .assert()
        .code(0)
        .stderr_eq("");
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    serde_json::from_str(&stdout).expect("stdout is one JSON document")
}

/// The one input of a document, which names one file.
fn only_input(doc: &Value) -> &Value {
    let inputs = doc["inputs"].as_array().expect("an `inputs` list");
    assert_eq!(inputs.len(), 1, "{doc}");
    &inputs[0]
}

/// The entry for `key` among the settings of `input`.
fn setting<'a>(input: &'a Value, key: &str) -> &'a Value {
    input["settings"]
        .as_array()
        .expect("a `settings` list")
        .iter()
        .find(|s| s["key"] == json!(key))
        .unwrap_or_else(|| panic!("no `{key}` setting in {input}"))
}

#[test]
fn explain_config_prints_the_config_and_each_setting_of_a_file_as_json() {
    let dir = project();
    let doc = explained(dir.path(), &["docs/a.md"]);
    assert_eq!(doc["schema_version"], json!(3), "{doc}");
    assert!(
        doc["slopvac_version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "{doc}"
    );
    let input = only_input(&doc);
    assert_eq!(input["path"], json!("docs/a.md"), "{doc}");
    assert_eq!(input["config_root"], json!("."), "{doc}");
    assert_eq!(
        input["config_sources"],
        json!([
            {"layer": "core"},
            {"layer": "project", "file": "slopvac.toml"},
        ]),
        "{doc}"
    );
    assert_eq!(input["profile"], json!("normal"), "{doc}");
    assert_eq!(
        input["genre"],
        json!({"id": "reference", "source": "default"}),
        "{doc}"
    );
    assert_eq!(input["front_matter_resolved"], json!(false), "{doc}");
    assert_eq!(
        setting(input, "locale"),
        &json!({
            "key": "locale",
            "value": "en-US",
            "source": {"layer": "override", "file": "slopvac.toml", "line": 6, "column": 1},
            "replaced": [
                {
                    "value": "en-GB",
                    "source": {"layer": "project", "file": "slopvac.toml", "line": 2, "column": 1},
                },
            ],
        }),
        "{doc}"
    );
    // A core default has an origin with no `file` key at all.
    assert_eq!(
        setting(input, "profile.name"),
        &json!({
            "key": "profile.name",
            "value": "normal",
            "source": {"layer": "core"},
            "replaced": [],
        }),
        "{doc}"
    );
}

#[test]
fn explain_config_lists_replaced_values_highest_layer_first() {
    let config = "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n\n[[overrides]]\nfiles = [\"docs/a.md\"]\nlocale = \"en-CA\"\n";
    let dir = project_with(config);
    let doc = explained(dir.path(), &["docs/a.md"]);
    let locale = setting(only_input(&doc), "locale");
    assert_eq!(locale["value"], json!("en-CA"), "{doc}");
    let replaced: Vec<&Value> = locale["replaced"]
        .as_array()
        .expect("a `replaced` list")
        .iter()
        .map(|r| &r["value"])
        .collect();
    assert_eq!(replaced, [&json!("en-US"), &json!("en-GB")], "{doc}");
}

#[test]
fn explain_config_lists_the_settings_in_the_order_of_the_text_form() {
    let dir = project();
    let text = slopvac(dir.path())
        .args(["lint", "--explain-config", "docs/a.md"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&text).into_owned();
    let from_text: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with("config "))
        .filter_map(|l| l.split_once(" = ").map(|(key, _)| key))
        .collect();
    let doc = explained(dir.path(), &["docs/a.md"]);
    let from_json: Vec<&str> = only_input(&doc)["settings"]
        .as_array()
        .expect("a `settings` list")
        .iter()
        .map(|s| s["key"].as_str().expect("a key"))
        .collect();
    assert!(from_json.len() > 2, "{doc}");
    assert_eq!(from_json, from_text);
}

#[test]
fn explain_config_gives_each_input_its_own_record_in_argument_order() {
    let dir = project();
    let doc = explained(dir.path(), &["docs/a.md", "a.md"]);
    let inputs = doc["inputs"].as_array().expect("an `inputs` list");
    let paths: Vec<&Value> = inputs.iter().map(|i| &i["path"]).collect();
    assert_eq!(paths, [&json!("docs/a.md"), &json!("a.md")], "{doc}");
    assert_eq!(setting(&inputs[0], "locale")["value"], json!("en-US"));
    assert_eq!(setting(&inputs[1], "locale")["value"], json!("en-GB"));
}

#[test]
fn explain_config_names_standard_input_stdin() {
    let dir = project();
    let assert = slopvac(dir.path())
        .args([
            "lint",
            "--explain-config",
            "--format",
            "json",
            "--stdin",
            "--language",
            "markdown",
        ])
        .stdin("# Title\n\nText.\n")
        .assert()
        .code(0)
        .stderr_eq("");
    let doc: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout is one JSON document");
    let input = only_input(&doc);
    assert_eq!(input["path"], json!("<stdin>"), "{doc}");
    assert_eq!(input["config_root"], json!("."), "{doc}");
}

#[test]
fn explain_config_shows_a_config_root_above_the_working_directory_as_external() {
    let dir = project();
    fs::create_dir(dir.path().join("sub")).expect("sub dir");
    fs::write(dir.path().join("sub/b.md"), "# Title\n\nText.\n").expect("write sub/b.md");
    let doc = explained(&dir.path().join("sub"), &["b.md"]);
    let name = dir.path().file_name().expect("name").to_string_lossy();
    let input = only_input(&doc);
    assert_eq!(
        input["config_root"],
        json!(format!("<external>/{name}")),
        "{doc}"
    );
    let parent = dir.path().parent().expect("parent").to_string_lossy();
    assert!(!doc.to_string().contains(parent.as_ref()), "{doc}");
}

#[test]
fn explain_config_shows_a_nested_config_root_relative_to_the_working_directory() {
    let dir = project();
    fs::write(
        dir.path().join("docs/slopvac.toml"),
        "schema_version = 3\nlocale = \"en-US\"\n",
    )
    .expect("write nested config");
    let doc = explained(dir.path(), &["docs/a.md"]);
    let input = only_input(&doc);
    assert_eq!(input["config_root"], json!("docs"), "{doc}");
    assert_eq!(
        input["config_sources"],
        json!([
            {"layer": "core"},
            {"layer": "project", "file": "slopvac.toml"},
        ]),
        "{doc}"
    );
}

#[test]
fn explain_config_prints_a_url_value_without_its_user_info_query_and_fragment() {
    let dir = project_with(
        "schema_version = 3\n[terms]\nallow = [\"https://user:SECRET@host/x?token=SECRET#SECRET\"]\n",
    );
    let doc = explained(dir.path(), &["a.md"]);
    assert!(!doc.to_string().contains("SECRET"), "{doc}");
    assert_eq!(
        setting(only_input(&doc), "terms.allow")["value"],
        json!(["https://host/x"]),
        "{doc}"
    );
}

#[test]
fn explain_config_does_not_read_front_matter_and_prints_none_of_it() {
    let dir = project();
    fs::write(
        dir.path().join("a.md"),
        "---\nlocale: FRONT-MATTER-SECRET\n---\n# Title\n\nText.\n",
    )
    .expect("write document");
    let doc = explained(dir.path(), &["a.md"]);
    assert!(!doc.to_string().contains("FRONT-MATTER-SECRET"), "{doc}");
    let input = only_input(&doc);
    assert_eq!(setting(input, "locale")["value"], json!("en-GB"), "{doc}");
    assert_eq!(input["front_matter_resolved"], json!(false), "{doc}");
}

#[test]
fn explain_config_lists_each_rule_a_layer_set_with_its_qualified_id_level_and_origin() {
    let dir = project_with(
        "schema_version = 3\n\n[rules]\n\"core:ste-practices.latin-abbreviation\" = \"error\"\n\"core:prose-craft.first-person-plural\" = \"off\"\n",
    );
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(
        input["configured_rules"],
        json!([
            {
                "id": "core:prose-craft.first-person-plural",
                "level": "off",
                "source": {"layer": "project", "file": "slopvac.toml", "line": 5, "column": 1},
            },
            {
                "id": "core:ste-practices.latin-abbreviation",
                "level": "error",
                "source": {"layer": "project", "file": "slopvac.toml", "line": 4, "column": 1},
            },
        ]),
        "{doc}"
    );
    let rules = &input["rules"];
    assert!(rules["total"].as_u64().is_some_and(|n| n > 2), "{doc}");
    assert_eq!(
        rules["configured"],
        json!({"error": 1, "warning": 0, "suggestion": 0, "off": 1}),
        "{doc}"
    );
}

#[test]
fn explain_config_lists_no_configured_rule_for_a_config_that_names_none() {
    let dir = project();
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(input["configured_rules"], json!([]), "{doc}");
    assert_eq!(
        input["rules"]["configured"],
        json!({"error": 0, "warning": 0, "suggestion": 0, "off": 0}),
        "{doc}"
    );
}

#[rstest]
#[case::min_score(&["--min-score", "50"], "--min-score")]
#[case::locale(&["--locale", "en-GB"], "--locale")]
#[case::disable(&["--disable", "prose-craft"], "--disable")]
fn explain_config_still_refuses_an_option_it_would_ignore_with_json(
    #[case] args: &[&str],
    #[case] option: &str,
) {
    let dir = project();
    let assert = slopvac(dir.path())
        .args(["lint", "--explain-config", "--format", "json", "a.md"])
        .args(args)
        .assert()
        .code(2)
        .stdout_eq("");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert_eq!(
        stderr,
        format!(
            "error[explain-config-option-unsupported]: `--explain-config` cannot be used with `{option}`; remove it.\n"
        )
    );
}

/// What a command printed: exit code, stdout and stderr.
fn outcome(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = slopvac(dir).args(args).output().expect("the binary runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A config that cannot be read or parsed stops `--explain-config --format json` the way it
/// stops `lint --format json`: same exit code, same stdout, same stderr.
#[rstest]
#[case::invalid_toml("slopvac.toml", b"schema_version = 3\nlocale = \n".as_slice())]
#[case::not_utf8("slopvac.toml", [0xff, 0xfe, 0x00].as_slice())]
fn an_unreadable_config_stops_explain_config_as_it_stops_lint_with_json(
    #[case] name: &str,
    #[case] bytes: &[u8],
) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join(name), bytes).expect("write config");
    fs::write(dir.path().join("a.md"), "# Title\n\nText.\n").expect("write document");
    let lint = outcome(dir.path(), &["lint", "--format", "json", "a.md"]);
    let explain = outcome(
        dir.path(),
        &["lint", "--explain-config", "--format", "json", "a.md"],
    );
    assert_eq!(lint.0, Some(2), "{lint:?}");
    assert!(lint.2.starts_with("error["), "{lint:?}");
    assert_eq!(explain, lint);
}

/// The same holds for a config in a directory below the working directory, which only the
/// engine reads (the working directory's own config is read before any command runs).
#[test]
fn an_unreadable_nested_config_stops_explain_config_as_it_stops_lint_with_json() {
    let dir = project();
    fs::write(
        dir.path().join("docs/slopvac.toml"),
        "schema_version = 3\nlocale = \n",
    )
    .expect("write nested config");
    let lint = outcome(dir.path(), &["lint", "--format", "json", "docs/a.md"]);
    let explain = outcome(
        dir.path(),
        &["lint", "--explain-config", "--format", "json", "docs/a.md"],
    );
    assert_eq!(lint.0, Some(2), "{lint:?}");
    let report: Value = serde_json::from_str(&lint.1).expect("lint prints its report");
    assert_eq!(
        report["notices"][0]["code"],
        json!("config-parse"),
        "{lint:?}"
    );
    assert_eq!(explain, lint);
}

/// An input that cannot be read is left out, as in the text form: the run exits 2 with the
/// error on stderr, and stdout still holds a document for the inputs that resolved.
#[test]
fn an_unknown_path_is_left_out_of_the_document_and_ends_the_run_with_exit_2() {
    let dir = project();
    let (code, stdout, stderr) = outcome(
        dir.path(),
        &[
            "lint",
            "--explain-config",
            "--format",
            "json",
            "missing.md",
            "a.md",
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert_eq!(stderr, "error[input-unreadable]: missing.md: not found\n");
    let doc: Value = serde_json::from_str(&stdout).expect("stdout is one JSON document");
    assert_eq!(doc["schema_version"], json!(3), "{doc}");
    assert_eq!(only_input(&doc)["path"], json!("a.md"), "{doc}");
}
