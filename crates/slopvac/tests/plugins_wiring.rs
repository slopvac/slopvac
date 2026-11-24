//! `slopvac lint` with plugins from the config: the lock pins each one and the store holds its
//! tree (docs/contract/plugins.md, sections 5 to 7; threat model X6).
//!
//! The store is laid out by hand as `plugins sync` leaves it (`trees/tree-<hex>` and
//! `refs/<digest hex>`), so the tests need no registry and no signing.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_plugins::extract::tree_hash;
use snapbox::cmd::{Command, cargo_bin};

/// A project and a per-user data directory, both private to one test.
pub(crate) struct World {
    pub(crate) dir: tempfile::TempDir,
    entries: Vec<String>,
}

impl World {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        for sub in [
            "data/slopvac/trees",
            "data/slopvac/refs",
            "stage",
            "project/.slopvac",
        ] {
            fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
        }
        Self {
            dir,
            entries: Vec::new(),
        }
    }

    pub(crate) fn project(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// Install a data plugin `id` with `files` and a `provides` line, and pin it in the lock.
    /// Returns the installed tree.
    pub(crate) fn install(
        &mut self,
        id: &str,
        provides: &str,
        files: &[(String, String)],
    ) -> PathBuf {
        let tree = self.dir.path().join("stage").join(id);
        fs::create_dir_all(&tree).expect("mkdir");
        fs::write(
            tree.join("slopvac-plugin.toml"),
            format!(
                "schema_version = 3\nid = \"{id}\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\n{provides}\n"
            ),
        )
        .expect("manifest");
        for (name, text) in files {
            let target = tree.join(name);
            fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
            fs::write(target, text).expect("file");
        }
        let hash = tree_hash(&tree).expect("hash");
        let dest = self
            .dir
            .path()
            .join("data/slopvac/trees")
            .join(format!("tree-{}", hash.hex()));
        fs::rename(&tree, &dest).expect("install");
        let digest = format!("{:064x}", self.entries.len() + 1);
        fs::write(
            self.dir.path().join("data/slopvac/refs").join(&digest),
            hash.hex(),
        )
        .expect("ref");
        self.entries.push(format!(
            "\n[[plugins]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\nsource = \"dist/{id}.tar.gz\"\ndigest = \"sha256:{digest}\"\nrequires = []\n"
        ));
        dest
    }

    /// Write the lock, the config that lists `ids` and a document with `text`.
    pub(crate) fn write_project(&self, ids: &[&str], text: &str) -> PathBuf {
        let project = self.project();
        fs::write(
            project.join(".slopvac/plugins.lock"),
            format!(
                "schema_version = 3\n{}",
                if self.entries.is_empty() {
                    "plugins = []\n".to_owned()
                } else {
                    self.entries.concat()
                }
            ),
        )
        .expect("lock");
        let listed = ids
            .iter()
            .map(|id| format!("\n[[plugins]]\nid = \"{id}\"\nsource = \"dist/{id}.tar.gz\"\n"))
            .collect::<Vec<_>>()
            .concat();
        fs::write(
            project.join("slopvac.toml"),
            format!("schema_version = 3\n{listed}"),
        )
        .expect("config");
        let doc = project.join("doc.md");
        fs::write(&doc, text).expect("doc");
        doc
    }

    /// The binary with the environment of the test, run in `cwd`.
    fn command(&self, cwd: &Path) -> Command {
        Command::new(cargo_bin!("slopvac"))
            .env("SLOPVAC_NO_UPDATE_CHECK", "1")
            .env("NO_COLOR", "1")
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env_remove("SLOPVAC_OFFLINE")
            .env_remove("SLOPVAC_LOG")
            .current_dir(cwd)
    }

    /// `slopvac lint --format json <doc>` in the project.
    pub(crate) fn lint(&self, doc: &Path) -> std::process::Output {
        self.lint_in(&self.project(), doc.file_name().expect("file name"))
    }

    /// `slopvac lint --format json <target>` run in `cwd`.
    pub(crate) fn lint_in(
        &self,
        cwd: &Path,
        target: impl AsRef<std::ffi::OsStr>,
    ) -> std::process::Output {
        self.command(cwd)
            .args(["lint", "--format", "json"])
            .arg(target)
            .output()
            .expect("run")
    }

    /// `slopvac <args>` in the project.
    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command(&self.project())
            .args(args)
            .output()
            .expect("run")
    }
}

/// A category file `c` with one pattern rule for each pattern, as the pack contract writes it.
pub(crate) fn category(patterns: &[String]) -> (String, String) {
    let rules: Vec<String> = patterns
        .iter()
        .enumerate()
        .map(|(i, pattern)| {
            format!(
                r#"{{"id":"r{i}","name":"N","message":"m","match":{{"kind":"pattern","pattern":"{pattern}"}},"citation":{{"source":"s"}}}}"#
            )
        })
        .collect();
    (
        "categories/c.json".to_owned(),
        format!(
            r#"{{"schema_version":3,"id":"c","title":"T","description":"D","rules":[{}]}}"#,
            rules.join(",")
        ),
    )
}

const PROVIDES_C: &str = "categories = [\"categories/c.json\"]";

fn rule_ids(report: &Value) -> Vec<String> {
    report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule_id"].as_str().expect("rule id").to_owned())
        .collect()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_rule_of_a_listed_plugin_reports_under_its_qualified_id() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nThe zzqx is here.\n");

    let out = w.lint(&doc);

    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert!(
        rule_ids(&report).contains(&"acme:c.r0".to_owned()),
        "{report}"
    );
}

#[test]
fn a_plugin_that_is_installed_but_not_listed_does_not_load() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&[], "# Title\n\nThe zzqx is here.\n");

    let out = w.lint(&doc);

    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert!(
        !rule_ids(&report).iter().any(|id| id.starts_with("acme:")),
        "{report}"
    );
}

#[test]
fn a_listed_plugin_that_the_lock_lacks_stops_the_run_with_exit_2() {
    let w = World::new();
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("error[plugin-not-locked]"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_tampered_tree_stops_the_run_with_exit_2() {
    let mut w = World::new();
    let tree = w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nThe zzqx is here.\n");
    let manifest = tree.join("slopvac-plugin.toml");
    let mut text = fs::read_to_string(&manifest).expect("read");
    text.push_str("# changed\n");
    fs::write(&manifest, text).expect("tamper");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("error[plugin-tampered]"),
        "{}",
        stderr(&out)
    );
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// Finding: a malformed lock names the file and where it is faulty, never the value there.
#[test]
fn a_lock_with_an_invalid_version_is_refused_without_echoing_the_value() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");
    let lock = w.project().join(".slopvac/plugins.lock");
    let text = fs::read_to_string(&lock).expect("lock");
    fs::write(
        &lock,
        text.replace("version = \"1.0.0\"", "version = \"hunter2-SECRET\""),
    )
    .expect("lock");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    let all = format!("{}{}", stderr(&out), stdout(&out));
    assert!(!all.contains("SECRET"), "{all}");
    assert!(
        stderr(&out).contains("error[run-plugins-lock-invalid]: `.slopvac/plugins.lock` is invalid at `/plugins/0/version`."),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_lock_that_is_a_directory_stops_the_run_with_exit_2() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");
    let lock = w.project().join(".slopvac/plugins.lock");
    fs::remove_file(&lock).expect("rm");
    fs::create_dir(&lock).expect("mkdir");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains(
            "error[run-plugins-lock-unreadable]: `.slopvac/plugins.lock` is not a regular file."
        ),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_locked_plugin_with_no_stored_tree_stops_the_run_with_exit_2() {
    let mut w = World::new();
    let tree = w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");
    fs::remove_dir_all(&tree).expect("remove the tree");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("error[plugin-not-installed]"),
        "{}",
        stderr(&out)
    );
}

// Policies section 1.9: an authored tagger pattern over 8,192 bytes.
#[test]
fn a_tagger_pattern_over_the_cap_stops_the_run_with_its_own_code() {
    let mut w = World::new();
    w.install(
        "acme",
        "taggers = [\"taggers/t.yaml\"]",
        &[(
            "taggers/t.yaml".to_owned(),
            format!(
                "schema_version: 3\nid: t\ntag: acme-tag\nmatch:\n  - pattern: '{}'\n",
                "a".repeat(8_193)
            ),
        )],
    );
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("error[tagger-pattern-too-long]"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("8193"), "{}", stderr(&out));
}

// Policies section 1.9: 256 files in a plugin is the most; 257 is `rules-budget-exceeded` with
// the budget and its limit in the report data.
#[test]
fn a_plugin_with_257_files_is_refused_with_its_budget_and_limit() {
    let mut w = World::new();
    let files: Vec<(String, String)> = (0..257)
        .map(|i| (format!("styles/s{i}.toml"), String::new()))
        .collect();
    w.install("acme", "styles = [\"styles/*.toml\"]", &files);
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let notice = &report["notices"][0];
    assert_eq!(notice["code"], "rules-budget-exceeded", "{report}");
    assert_eq!(notice["data"]["budget"], "plugin-files", "{report}");
    assert_eq!(notice["data"]["limit"], 256, "{report}");
}

#[test]
fn a_rule_with_4097_tokens_is_refused_with_its_budget_and_limit() {
    let mut w = World::new();
    let tokens: Vec<String> = (0..4097).map(|i| format!("t{i}")).collect();
    let rule = serde_json::json!({
        "id": "r", "name": "n", "message": "m",
        "match": {"kind": "tokens", "tokens": tokens},
        "citation": {"source": "s"}
    });
    let text = serde_json::json!({
        "schema_version": 3, "id": "c", "title": "T", "description": "D", "rules": [rule]
    })
    .to_string();
    w.install(
        "acme",
        PROVIDES_C,
        &[("categories/c.json".to_owned(), text)],
    );
    let doc = w.write_project(&["acme"], "# Title\n\nText.\n");

    let out = w.lint(&doc);

    assert_eq!(out.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let notice = &report["notices"][0];
    assert_eq!(notice["code"], "rules-budget-exceeded", "{report}");
    assert_eq!(notice["data"]["budget"], "rule-entries", "{report}");
    assert_eq!(notice["data"]["limit"], 4096, "{report}");
}

// Policies section 1.9: a category file may hold up to 2 MiB.
#[test]
fn a_category_of_one_and_a_half_mebibytes_loads() {
    let mut w = World::new();
    let (name, mut text) = category(&["zzqx".to_owned()]);
    text.insert_str(
        text.len() - 1,
        &format!(",\"x-pad\":\"{}\"", "p".repeat(1_536 * 1024)),
    );
    w.install("acme", PROVIDES_C, &[(name, text)]);
    let doc = w.write_project(&["acme"], "# Title\n\nThe zzqx is here.\n");

    let out = w.lint(&doc);

    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert!(
        rule_ids(&report).contains(&"acme:c.r0".to_owned()),
        "{report}"
    );
}

// Run from a subdirectory, the plugins and the lock come from the config that applies there.
#[test]
fn a_run_from_a_subdirectory_loads_the_plugins_of_the_parent_config() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    w.write_project(&["acme"], "# Title\n\nText.\n");
    let sub = w.project().join("docs");
    fs::create_dir(&sub).expect("mkdir");
    fs::write(sub.join("d.md"), "# Title\n\nThe zzqx is here.\n").expect("doc");

    let out = w.lint_in(&sub, "d.md");

    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert!(
        rule_ids(&report).contains(&"acme:c.r0".to_owned()),
        "{report}"
    );
}

// A nested config that lists a plugin the run did not load is refused, not linted without it.
#[test]
fn a_nested_config_that_lists_more_plugins_is_refused() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    w.write_project(&[], "# Title\n\nText.\n");
    let sub = w.project().join("docs");
    fs::create_dir(&sub).expect("mkdir");
    fs::write(
        sub.join("slopvac.toml"),
        "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nsource = \"dist/acme.tar.gz\"\n",
    )
    .expect("nested config");
    fs::write(sub.join("d.md"), "# Title\n\nThe zzqx is here.\n").expect("doc");

    let out = w.lint_in(&w.project(), "docs/d.md");

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("error[plugins-nested-config]"),
        "{}",
        stderr(&out)
    );
}

// A plugin may not supply a category or rule id that an earlier pack supplies (policies and
// plugins contract, rule ids): the engine keys category settings by that id.
#[test]
fn a_plugin_category_that_shadows_a_core_category_is_refused() {
    let mut w = World::new();
    let rules = r#"{"schema_version":3,"id":"orwell","title":"T","description":"D","rules":[{"id":"r0","name":"N","message":"m","match":{"kind":"pattern","pattern":"zzqx"},"citation":{"source":"s"}}]}"#;
    w.install(
        "acme",
        "categories = [\"categories/o.json\"]",
        &[("categories/o.json".to_owned(), rules.to_owned())],
    );
    let doc = w.write_project(&["acme"], "# Title\n\nThe zzqx is here.\n");

    let out = w.run(&[
        "lint",
        doc.file_name().and_then(|n| n.to_str()).expect("name"),
    ]);

    // Finding 2 (lead ruling, contract comment 7118656459): refused at load with its own code.
    // The core has the category `orwell`, so the shared id is the category id.
    assert_eq!(out.status.code(), Some(2));
    let expected = "error[pack-rule-id-collision]: plugin `acme` is refused: its rule `orwell` has the id of a rule that another pack supplies, and nothing from the plugin loads";
    assert!(
        stderr(&out).lines().any(|l| l == expected),
        "{}",
        stderr(&out)
    );
    // Not asserted: empty stdout. A refused run prints the generic incomplete-run report on
    // stdout (`notice[<code>]` and `0 documents`), as every refusal does; see the PR comment.
}

#[test]
fn a_plugin_whose_ids_do_not_collide_still_loads() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    let doc = w.write_project(&["acme"], "# Title\n\nThe zzqx is here.\n");

    let out = w.lint(&doc);

    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert!(
        rule_ids(&report).contains(&"acme:c.r0".to_owned()),
        "{report}"
    );
}

// `explain` names every rule that `lint` reports.
#[test]
fn explain_resolves_a_rule_of_a_listed_plugin() {
    let mut w = World::new();
    w.install("acme", PROVIDES_C, &[category(&["zzqx".to_owned()])]);
    w.write_project(&["acme"], "# Title\n\nText.\n");

    let out = w.run(&["explain", "acme:c.r0"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("acme:c.r0"), "{}", stdout(&out));
}
