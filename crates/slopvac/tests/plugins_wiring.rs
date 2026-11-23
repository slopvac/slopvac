//! `slopvac lint` with plugins from the config: the lock pins each one and the store holds its
//! tree (docs/contract/plugins.md, sections 5 to 7; threat model X6).
//!
//! The store is laid out by hand as `plugins sync` leaves it (`trees/tree-<hex>` and
//! `refs/<digest hex>`), so the tests need no registry and no signing.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use slopvac_plugins::extract::tree_hash;

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

    pub(crate) fn lint(&self, doc: &Path) -> std::process::Output {
        let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
        cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1")
            .env("NO_COLOR", "1")
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env_remove("SLOPVAC_OFFLINE")
            .env_remove("SLOPVAC_LOG")
            .current_dir(self.project());
        cmd.args(["lint", "--format", "json"])
            .arg(doc.file_name().expect("file name"))
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
