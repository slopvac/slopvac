//! `slopvac lint --explain-config --format json` prints the resolved settings of each input as
//! one JSON document (`schemas/v3/explain-config.schema.json`, `docs/contract/config.md`
//! section 1.4.1).
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

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
        json!({"id": "consumer", "source": "default"}),
        "{doc}"
    );
    assert_eq!(input["front_matter_resolved"], json!(true), "{doc}");
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
    // The default genre `consumer` sets the profile in its genre file (layer 3), over the core
    // default, whose origin has no `file` key at all.
    assert_eq!(
        setting(input, "profile.name"),
        &json!({
            "key": "profile.name",
            "value": "normal",
            "source": {"layer": "genre", "file": "plugin:core/genres/consumer.yaml", "line": 4, "column": 1},
            "replaced": [
                {"value": "normal", "source": {"layer": "core"}},
            ],
        }),
        "{doc}"
    );
}

#[test]
fn explain_config_lists_replaced_values_highest_layer_first() {
    let config = "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"und\"\n\n[[overrides]]\nfiles = [\"docs/a.md\"]\nlocale = \"en-US\"\n";
    let dir = project_with(config);
    let doc = explained(dir.path(), &["docs/a.md"]);
    let locale = setting(only_input(&doc), "locale");
    assert_eq!(locale["value"], json!("en-US"), "{doc}");
    let replaced: Vec<&Value> = locale["replaced"]
        .as_array()
        .expect("a `replaced` list")
        .iter()
        .map(|r| &r["value"])
        .collect();
    assert_eq!(replaced, [&json!("und"), &json!("en-GB")], "{doc}");
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
fn explain_config_reads_no_front_matter_key_outside_the_slopvac_block() {
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
    assert_eq!(input["front_matter_resolved"], json!(true), "{doc}");
}

#[test]
fn explain_config_lists_each_rule_a_layer_set_with_its_qualified_id_level_and_origin() {
    let dir = project_with(
        "schema_version = 3\n\n[rules]\n\"core:ste-practices.latin-abbreviation\" = \"error\"\n\"core:prose-craft.first-person-plural\" = \"off\"\n",
    );
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    // `a.md` has the default genre `consumer`, and its genre file is a layer too.
    assert_eq!(
        input["configured_rules"],
        json!([
            {
                "id": "core:docs-discipline.history-internal-reference",
                "level": "warning",
                "source": {"layer": "genre", "file": "plugin:core/genres/consumer.yaml", "line": 12, "column": 3},
            },
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
        json!({"error": 1, "warning": 1, "suggestion": 0, "off": 1}),
        "{doc}"
    );
}

/// A config that names no rule configures none: the one configured rule of `a.md` comes from
/// the genre file of its default genre `consumer`.
#[test]
fn explain_config_lists_no_configured_rule_for_a_config_that_names_none() {
    let dir = project();
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(
        input["configured_rules"],
        json!([{
            "id": "core:docs-discipline.history-internal-reference",
            "level": "warning",
            "source": {"layer": "genre", "file": "plugin:core/genres/consumer.yaml", "line": 12, "column": 3},
        }]),
        "{doc}"
    );
    assert_eq!(
        input["rules"]["configured"],
        json!({"error": 0, "warning": 1, "suggestion": 0, "off": 0}),
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

/// A config file that `extends` a file outside the project root shows that file as
/// `<external>/<file name>` in `config_sources` and in every origin, never with `..` or the
/// directories above the root (policies.md, section 1.7).
#[test]
fn explain_config_shows_an_extended_config_outside_the_root_as_external() {
    let outer = tempfile::tempdir().expect("tempdir");
    let proj = outer.path().join("proj");
    let shared = outer.path().join("zz-shared-dir");
    fs::create_dir_all(&proj).expect("proj");
    fs::create_dir_all(&shared).expect("shared");
    fs::write(
        proj.join("slopvac.toml"),
        "schema_version = 3\nextends = [\"../zz-shared-dir/base.toml\"]\n",
    )
    .expect("write config");
    fs::write(
        shared.join("base.toml"),
        "schema_version = 3\nlocale = \"en-GB\"\n",
    )
    .expect("write extended config");
    fs::write(proj.join("a.md"), "# Title\n\nText.\n").expect("write document");
    let doc = explained(&proj, &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(
        input["config_sources"],
        json!([
            {"layer": "core"},
            {"layer": "extends", "file": "<external>/base.toml"},
            {"layer": "project", "file": "slopvac.toml"},
        ]),
        "{doc}"
    );
    assert_eq!(
        setting(input, "locale")["source"],
        json!({"layer": "extends", "file": "<external>/base.toml", "line": 2, "column": 1}),
        "{doc}"
    );
    let text = doc.to_string();
    assert!(!text.contains("zz-shared-dir"), "{doc}");
    assert!(!text.contains("../"), "{doc}");
}

/// A `slopvac.toml` above the working directory that cannot be read is found by the update
/// policy check, which runs before any command. It stops `--explain-config` as it stops `lint`.
#[test]
fn an_unreadable_parent_config_stops_explain_config_as_it_stops_lint_with_json() {
    let outer = tempfile::tempdir().expect("tempdir");
    fs::write(outer.path().join("slopvac.toml"), [0xff, 0xfe, 0x00]).expect("write config");
    let sub = outer.path().join("sub");
    fs::create_dir(&sub).expect("sub dir");
    fs::write(sub.join("a.md"), "# Title\n\nText.\n").expect("write document");
    let lint = outcome(&sub, &["lint", "--format", "json", "a.md"]);
    let explain = outcome(
        &sub,
        &["lint", "--explain-config", "--format", "json", "a.md"],
    );
    assert_eq!(lint.0, Some(2), "{lint:?}");
    assert_eq!(lint.1, "", "{lint:?}");
    assert!(
        lint.2
            .starts_with("error[config-unreadable]: <external>/slopvac.toml: "),
        "{lint:?}"
    );
    assert_eq!(explain, lint);
}
/// The layers of every origin in the record `input`: its settings, the values they replaced and
/// its configured rules.
fn origin_layers(input: &Value) -> Vec<String> {
    let mut layers = Vec::new();
    for entry in input["settings"].as_array().expect("a `settings` list") {
        layers.push(entry["source"]["layer"].clone());
        for replaced in entry["replaced"].as_array().expect("a `replaced` list") {
            layers.push(replaced["source"]["layer"].clone());
        }
    }
    for rule in input["configured_rules"].as_array().expect("a rule list") {
        layers.push(rule["source"]["layer"].clone());
    }
    layers
        .into_iter()
        .map(|l| l.as_str().expect("a layer name").to_owned())
        .collect()
}

/// A document whose `slopvac:` block sets `terms.allow` on line 5 (its key at column 5) and
/// whose directive sets `locale` on line 9. `<!-- slopvac: ` is 14 bytes, so that key starts at
/// column 15.
const LAYERED: &str = "---\nslopvac:\n  v: 3\n  terms:\n    allow: [zorblax]\n---\n# Title\n\n<!-- slopvac: locale=en-US -->\n";

/// A front-matter value and a directive value are settings of the record, each with the input
/// as its file and the line and column of its key (docs/contract/config.md, section 1.4.1).
#[test]
fn explain_config_shows_a_front_matter_and_a_directive_value_at_their_origin_with_json() {
    let dir = project_with("schema_version = 3\nlocale = \"en-GB\"\n");
    fs::write(dir.path().join("a.md"), LAYERED).expect("write document");
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(input["front_matter_resolved"], json!(true), "{doc}");
    let allow = setting(input, "terms.allow");
    assert_eq!(allow["value"], json!(["zorblax"]), "{doc}");
    assert_eq!(
        allow["source"],
        json!({"layer": "front-matter", "file": "a.md", "line": 5, "column": 5}),
        "{doc}"
    );
    let locale = setting(input, "locale");
    assert_eq!(locale["value"], json!("en-US"), "{doc}");
    assert_eq!(
        locale["source"],
        json!({"layer": "directive", "file": "a.md", "line": 9, "column": 15}),
        "{doc}"
    );
    assert_eq!(
        locale["replaced"],
        json!([{
            "value": "en-GB",
            "source": {"layer": "project", "file": "slopvac.toml", "line": 2, "column": 1},
        }]),
        "{doc}"
    );
}

/// The record names the genre that lint gives the same input, with the same source
/// (docs/contract/config.md, section 1.4.1).
#[rstest]
#[case::front_matter(
    "schema_version = 3\n",
    "a.md",
    "---\nslopvac:\n  v: 3\n  genre: informal\n---\n# Title\n\nText.\n",
    "explicit"
)]
#[case::directive(
    "schema_version = 3\n",
    "a.md",
    "<!-- slopvac: genre=informal -->\n# Title\n\nText.\n",
    "explicit"
)]
#[case::genres_block(
    "schema_version = 3\n\n[[genres]]\nfiles = [\"**\"]\ngenre = \"internal\"\n",
    "a.md",
    "# Title\n\nText.\n",
    "explicit"
)]
#[case::detection(
    "schema_version = 3\n",
    "reference/e.md",
    "# Title\n\nText.\n",
    "detected"
)]
#[case::none("schema_version = 3\n", "a.md", "# Title\n\nText.\n", "default")]
fn explain_config_names_the_genre_that_lint_reports_for_the_input(
    #[case] config: &str,
    #[case] path: &str,
    #[case] text: &str,
    #[case] source: &str,
) {
    let dir = project_with(config);
    let file = dir.path().join(path);
    fs::create_dir_all(file.parent().expect("a parent")).expect("dir");
    fs::write(&file, text).expect("write document");
    let doc = explained(dir.path(), &[path]);
    let (code, stdout, stderr) = outcome(dir.path(), &["lint", "--format", "json", path]);
    assert!(code.is_some_and(|c| c < 2), "{stderr}");
    let report: Value = serde_json::from_str(&stdout).expect("a lint report");
    let linted = &report["documents"][0]["genre"];
    let explained = &only_input(&doc)["genre"];
    assert_eq!(explained["source"], json!(source), "{doc}");
    assert_eq!(explained["id"], linted["id"], "{doc}\n{report}");
    assert_eq!(explained["source"], linted["source"], "{doc}\n{report}");
}

/// A genre id that is well formed and names no registered genre. It must never be echoed.
const UNREGISTERED: &str = "zzsecretgenre";

/// A document that names a genre no registry holds is refused, by lint and by explain alike,
/// with `config-invalid-value` at its key. The refusal never carries the value, neither on
/// stderr nor in the report, and the other document of the run is still read. Explain prints
/// no record for the refused input, in text or JSON (docs/contract/config.md, section 1.4.1).
///
/// The stderr line names the key: in the front matter, `genre` is on line 4 at column 3; in
/// the directive, it follows the 14 characters of `<!-- slopvac: `, at column 15.
#[rstest]
#[case::front_matter(
    "---\nslopvac:\n  v: 3\n  genre: zzsecretgenre\n---\n# Title\n\nText.\n",
    "a.md:4:3"
)]
#[case::directive(
    "<!-- slopvac: genre=zzsecretgenre -->\n# Title\n\nText.\n",
    "a.md:1:15"
)]
fn an_unregistered_genre_is_refused_by_lint_and_explain_without_its_value(
    #[case] text: &str,
    #[case] location: &str,
) {
    let dir = project_with("schema_version = 3\n");
    fs::write(dir.path().join("a.md"), text).expect("write document");

    let (code, stdout, stderr) = outcome(dir.path(), &["lint", "--format", "json", "."]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("error[config-invalid-value]"), "{stderr}");
    assert!(stderr.contains(location), "{stderr}");
    assert!(!stderr.contains(UNREGISTERED), "{stderr}");
    assert!(!stdout.contains(UNREGISTERED), "{stdout}");
    let report: Value = serde_json::from_str(&stdout).expect("a lint report");
    let paths: Vec<&Value> = report["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|d| &d["path"])
        .collect();
    assert_eq!(paths, [&json!("docs/a.md")], "{report}");

    let (code, stdout, stderr) = outcome(
        dir.path(),
        &["lint", "--explain-config", "--format", "json", "."],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("error[config-invalid-value]"), "{stderr}");
    assert!(!stderr.contains(UNREGISTERED), "{stderr}");
    assert!(!stdout.contains(UNREGISTERED), "{stdout}");
    let refusal = stderr
        .lines()
        .find(|l| l.starts_with("error[config-invalid-value]"))
        .expect("a refusal line");
    assert!(refusal.contains(location), "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("one JSON document");
    assert_eq!(only_input(&doc)["path"], json!("docs/a.md"), "{doc}");

    // The text form prints one block per record, headed by the input's path.
    let (code, stdout, stderr) = outcome(dir.path(), &["lint", "--explain-config", "."]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains(location), "{stderr}");
    assert!(!stderr.contains(UNREGISTERED), "{stderr}");
    assert!(!stdout.contains(UNREGISTERED), "{stdout}");
    let headings: Vec<&str> = stdout.lines().filter(|l| l.ends_with("a.md")).collect();
    assert_eq!(headings, ["docs/a.md"], "{stdout}");
}

/// A document whose directive names an unregistered genre still has its other errors
/// reported: the unknown front-matter key `zorp` (line 4, column 3) and the refused genre
/// (line 6, at column 15 after `<!-- slopvac: `) both reach stderr, from lint and from explain
/// alike, and the run is incomplete.
#[rstest]
#[case::lint(&["lint", "--format", "json", "a.md"])]
#[case::explain(&["lint", "--explain-config", "--format", "json", "a.md"])]
fn a_refused_directive_genre_still_reports_a_front_matter_error(#[case] args: &[&str]) {
    let dir = project_with("schema_version = 3\n");
    fs::write(
        dir.path().join("a.md"),
        format!(
            "{}<!-- slopvac: genre={UNREGISTERED} -->\n# Title\n\nText.\n",
            "---\nslopvac:\n  v: 3\n  zorp: 1\n---\n"
        ),
    )
    .expect("write document");
    let (code, _, stderr) = outcome(dir.path(), args);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("error[config-unknown-key]: a.md:4:3: "),
        "{stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("error[config-invalid-value]") && l.contains("a.md:6:15")),
        "{stderr}"
    );
}

/// A document whose `[[genres]]` assignment names an unregistered genre does not take the
/// detected genre from a sibling under the same config: `reference/e.md` gets the genre and
/// the findings that it gets when it is linted alone, and only `a.md` is refused.
#[test]
fn an_unregistered_assignment_leaves_the_detected_genre_of_its_siblings() {
    let dir = project_with(&format!(
        "schema_version = 3\n\n[[genres]]\nfiles = [\"a.md\"]\ngenre = \"{UNREGISTERED}\"\n"
    ));
    fs::create_dir(dir.path().join("reference")).expect("reference dir");
    fs::write(
        dir.path().join("reference/e.md"),
        "# Notes\n\nWe refactored the parser.\n",
    )
    .expect("write document");
    let sibling = |report: &Value| {
        report["documents"]
            .as_array()
            .expect("documents")
            .iter()
            .find(|d| d["path"] == json!("reference/e.md"))
            .cloned()
            .unwrap_or_else(|| panic!("no reference/e.md in {report}"))
    };

    let (code, stdout, stderr) =
        outcome(dir.path(), &["lint", "--format", "json", "reference/e.md"]);
    assert!(code.is_some_and(|c| c < 2), "{stderr}");
    let alone = sibling(&serde_json::from_str(&stdout).expect("a lint report"));
    assert_eq!(alone["genre"]["source"], json!("detected"), "{alone}");

    let (code, stdout, stderr) = outcome(
        dir.path(),
        &["lint", "--format", "json", "a.md", "reference/e.md"],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("error[config-invalid-value]"), "{stderr}");
    let together = sibling(&serde_json::from_str(&stdout).expect("a lint report"));
    assert_eq!(together["genre"], alone["genre"], "{together}");
    assert_eq!(together["findings"], alone["findings"], "{together}");
}

/// With `[frontmatter] enabled = false` the document takes no part in resolution: the record
/// says so, and no origin is the `slopvac:` block or a directive.
#[test]
fn explain_config_with_front_matter_off_resolves_no_document_layer() {
    let dir = project_with("schema_version = 3\n\n[frontmatter]\nenabled = false\n");
    fs::write(dir.path().join("a.md"), LAYERED).expect("write document");
    let doc = explained(dir.path(), &["a.md"]);
    let input = only_input(&doc);
    assert_eq!(input["front_matter_resolved"], json!(false), "{doc}");
    let layers = origin_layers(input);
    assert!(
        !layers
            .iter()
            .any(|l| l == "front-matter" || l == "directive"),
        "{doc}"
    );
}

/// A `slopvac:` block with an error is reported on stderr at its key, the run is incomplete,
/// and the record is still printed without any value from that block. The unknown key `zorp`
/// is on line 5, at column 3.
#[test]
fn explain_config_reports_a_bad_block_and_keeps_the_record_without_its_values() {
    let dir = project_with("schema_version = 3\nlocale = \"en-GB\"\n");
    fs::write(
        dir.path().join("a.md"),
        "---\nslopvac:\n  v: 3\n  locale: en-US\n  zorp: 1\n---\n# Title\n\nText.\n",
    )
    .expect("write document");
    let (code, stdout, stderr) = outcome(
        dir.path(),
        &["lint", "--explain-config", "--format", "json", "a.md"],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.starts_with("error[config-unknown-key]: a.md:5:3: "),
        "{stderr}"
    );
    let doc: Value = serde_json::from_str(&stdout).expect("stdout is one JSON document");
    let input = only_input(&doc);
    assert_eq!(setting(input, "locale")["value"], json!("en-GB"), "{doc}");
    assert!(
        !origin_layers(input).iter().any(|l| l == "front-matter"),
        "{doc}"
    );
}

/// The markers of a steering block are no directive, so a steering file explains cleanly and
/// no origin is a directive (docs/contract/config.md, section 1.1.1).
#[test]
fn explain_config_reads_no_directive_from_a_steering_block() {
    let dir = project_with("schema_version = 3\n");
    fs::write(
        dir.path().join("AGENTS.md"),
        "# Agents\n\n<!-- slopvac:begin -->\nRun the checks.\n<!-- slopvac:end -->\n",
    )
    .expect("write document");
    let doc = explained(dir.path(), &["AGENTS.md"]);
    assert!(
        !origin_layers(only_input(&doc))
            .iter()
            .any(|l| l == "directive"),
        "{doc}"
    );
}
