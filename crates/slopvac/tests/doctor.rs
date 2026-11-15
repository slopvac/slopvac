//! Installed-binary tests for `slopvac doctor` (compat rows `cmd:doctor`, `opt:doctor:--format`,
//! `opt:doctor:--binary`, `opt:doctor:--config`; docs/contract/doctor.md).
//!
//! Every JSON document the binary prints in these tests is also validated against
//! `schemas/v3/doctor-report.schema.json`.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};

const LOCK: &str = include_str!("../../../schemas/v3/conformance/packs-lock/valid/full.toml");

/// An isolated world: a project directory, a home with no state, and a sentinel name that must
/// never reach the output.
struct World {
    root: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("sentinel-secret-")
            .tempdir()
            .unwrap();
        for dir in ["project", "home", "cache"] {
            fs::create_dir(root.path().join(dir)).unwrap();
        }
        Self { root }
    }

    fn project(&self) -> PathBuf {
        self.root.path().join("project")
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn cache(&self) -> PathBuf {
        self.root.path().join("cache")
    }

    fn sentinel(&self) -> String {
        self.root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.project().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// `slopvac doctor` with a clean environment.
    fn doctor(&self) -> Command {
        let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
        for var in [
            "SLOPVAC_OFFLINE",
            "SLOPVAC_LOG",
            "SLOPVAC_CACHE_DIR",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "LOCALAPPDATA",
            "CI",
            "NO_UPDATE_NOTIFIER",
            "SLOPVAC_NO_UPDATE_CHECK",
            "DO_NOT_TRACK",
            "FORCE_COLOR",
            "CLICOLOR_FORCE",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("NO_COLOR", "1")
            .env("HOME", self.home())
            .env("LOCALAPPDATA", self.home())
            .env("SLOPVAC_CACHE_DIR", self.cache())
            .current_dir(self.project())
            .arg("doctor");
        cmd
    }

    /// Run `doctor --format json`, check the schema and the sentinel, and return the exit code and
    /// the document.
    fn json(&self, extra: &[&str], tweak: impl FnOnce(&mut Command)) -> (i32, Value) {
        let mut cmd = self.doctor();
        cmd.args(["--format", "json"]).args(extra);
        tweak(&mut cmd);
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            !stdout.contains(&self.sentinel()),
            "an absolute path leaked:\n{stdout}"
        );
        let doc: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"));
        let errors: Vec<String> = validator()
            .iter_errors(&doc)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "schema errors: {errors:?}\n{stdout}");
        (output.status.code().unwrap(), doc)
    }
}

fn schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

/// The published schema, with its siblings registered for `$ref`.
fn validator() -> jsonschema::Validator {
    const BASE: &str = "https://srobroek.github.io/slopvac/schemas/v3";
    let mut registry = jsonschema::Registry::new();
    for entry in fs::read_dir(schemas()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            registry = registry.add(format!("{BASE}/{name}"), doc).unwrap();
        }
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&json!({ "$ref": format!("{BASE}/doctor-report.schema.json") }))
        .unwrap()
}

const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[test]
fn a_clean_project_reports_ok_with_the_documented_shape() {
    let w = World::new();
    let (code, doc) = w.json(&[], |_| {});
    assert_eq!(code, 0);
    assert_eq!(doc["schema_version"], 3);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["slopvac"]["version"], CURRENT);
    assert_eq!(doc["slopvac"]["install_method"], "unknown");
    assert_eq!(doc["platform"]["os"], std::env::consts::OS);
    assert_eq!(doc["platform"]["arch"], std::env::consts::ARCH);
    assert_eq!(doc["config"], json!({"ok": true, "sources": []}));
    assert_eq!(doc["packs"]["lockfile"]["status"], "absent");
    assert_eq!(doc["packs"]["items"], json!([]));
    assert_eq!(doc["trust"]["status"], "absent");
    assert_eq!(doc["offline"], json!({"effective": false, "sources": []}));
}

#[test]
fn text_is_the_default_format_and_ends_with_the_verdict() {
    let w = World::new();
    w.doctor()
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with(format!(
            "slopvac {CURRENT} (unknown) on "
        )))
        .stdout(predicate::str::ends_with("ok\n"))
        .stdout(predicate::str::contains(
            "config: none found, defaults apply",
        ))
        .stdout(predicate::str::contains("sentinel-secret-").not());
}

#[test]
fn the_removed_binary_option_is_a_usage_error() {
    let w = World::new();
    w.doctor()
        .args(["--binary", "vale"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
}

#[test]
fn config_is_discovered_upward_and_stops_at_the_git_root() {
    let w = World::new();
    w.write("slopvac.toml", "schema_version = 3\n");
    fs::create_dir(w.project().join("docs")).unwrap();
    let (code, doc) = w.json(&[], |c| {
        c.current_dir(w.project().join("docs"));
    });
    assert_eq!(code, 0);
    assert_eq!(
        doc["config"]["sources"][0],
        json!({"layer": "project", "file": "<external>/slopvac.toml", "status": "ok", "error": null})
    );

    // A .git directory in `docs` ends the walk before it reaches the parent's file.
    fs::create_dir(w.project().join("docs/.git")).unwrap();
    let (_, doc) = w.json(&[], |c| {
        c.current_dir(w.project().join("docs"));
    });
    assert_eq!(doc["config"]["sources"], json!([]));
}

#[test]
fn a_file_in_the_working_directory_is_shown_relative_to_it() {
    let w = World::new();
    w.write("slopvac.toml", "schema_version = 3\n");
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(doc["config"]["sources"][0]["file"], "slopvac.toml");
}

#[test]
fn the_config_option_names_one_file_and_skips_discovery() {
    let w = World::new();
    w.write("slopvac.toml", "{{{ not toml");
    w.write("ci/other.toml", "schema_version = 3\n");
    let (code, doc) = w.json(&["--config", "ci/other.toml"], |_| {});
    assert_eq!(code, 0);
    assert_eq!(
        doc["config"]["sources"],
        json!([{"layer": "explicit", "file": "ci/other.toml", "status": "ok", "error": null}])
    );
}

#[test]
fn a_missing_config_file_fails_the_check() {
    let w = World::new();
    let (code, doc) = w.json(&["--config", "nope.toml"], |_| {});
    assert_eq!(code, 2);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["config"]["sources"][0]["status"], "missing");
}

#[test]
fn broken_config_files_get_a_status_and_never_echo_their_content() {
    let w = World::new();
    for (body, status, needle) in [
        (
            "# 2.x\n[vale]\nbinary = \"vale\"\n",
            "legacy",
            "slopvac migrate",
        ),
        ("schema_version = 2\n", "legacy", "slopvac migrate"),
        (
            "schema_version = 4\n",
            "invalid",
            "unsupported schema_version",
        ),
        ("password = \"hunter2\"\n{{{\n", "invalid", "line 2"),
    ] {
        w.write("slopvac.toml", body);
        let (code, doc) = w.json(&[], |_| {});
        assert_eq!(code, 2, "{body}");
        assert_eq!(doc["config"]["sources"][0]["status"], status, "{body}");
        let error = doc["config"]["sources"][0]["error"].as_str().unwrap();
        assert!(error.contains(needle), "{error}");
        assert!(!error.contains("hunter2"), "{error}");
    }
}

#[test]
fn an_oversize_or_non_regular_config_is_unreadable() {
    let w = World::new();
    w.write("slopvac.toml", &"#".repeat(1024 * 1024 + 1));
    let (code, doc) = w.json(&[], |_| {});
    assert_eq!(code, 2);
    assert_eq!(doc["config"]["sources"][0]["status"], "unreadable");

    fs::remove_file(w.project().join("slopvac.toml")).unwrap();
    fs::create_dir(w.project().join("slopvac.toml")).unwrap();
    let (code, doc) = w.json(&[], |_| {});
    assert_eq!(code, 2);
    assert_eq!(doc["config"]["sources"][0]["error"], "not a regular file");
}

#[test]
fn offline_lists_every_source_that_turned_it_on() {
    let w = World::new();
    w.write(
        "slopvac.toml",
        "schema_version = 3\n[network]\noffline = true\n",
    );
    let (_, doc) = w.json(&["--offline"], |c| {
        c.env("SLOPVAC_OFFLINE", "1");
    });
    assert_eq!(
        doc["offline"],
        json!({"effective": true, "sources": ["flag", "env", "config"]})
    );
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(
        doc["offline"],
        json!({"effective": true, "sources": ["config"]})
    );
    w.write(
        "slopvac.toml",
        "schema_version = 3\n[network]\noffline = false\n",
    );
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(doc["offline"]["effective"], false);
}

fn cache_entry(w: &World, latest: &str) {
    fs::write(
        w.cache().join("update-check.json"),
        json!({"checked_at": 1_790_000_000u64, "latest": latest}).to_string(),
    )
    .unwrap();
}

#[test]
fn update_state_comes_from_the_cached_check() {
    let w = World::new();
    let (_, unknown) = w.json(&[], |_| {});
    assert_eq!(unknown["update"]["state"], "unknown");
    assert_eq!(unknown["update"]["latest"], Value::Null);

    for (latest, state) in [("99.0.0", "available"), (CURRENT, "current")] {
        cache_entry(&w, latest);
        let (code, doc) = w.json(&[], |_| {});
        assert_eq!(code, 0, "an available update is not a problem");
        assert_eq!(doc["update"]["state"], state, "{latest}");
        assert_eq!(doc["update"]["latest"], latest);
        assert_eq!(doc["update"]["checked_at"], 1_790_000_000u64);
        assert_eq!(doc["update"]["checks"], "enabled");
    }
    fs::write(w.cache().join("update-check.json"), "not json").unwrap();
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(doc["update"]["state"], "unknown");
}

#[test]
fn update_checks_report_why_they_are_off() {
    let w = World::new();
    w.write(
        "slopvac.toml",
        "schema_version = 3\n[network]\nupdate_check = false\n",
    );
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(doc["update"]["disabled_by"], "config");
    w.write("slopvac.toml", "schema_version = 3\n");

    for (var, why) in [
        ("DO_NOT_TRACK", "env"),
        ("NO_UPDATE_NOTIFIER", "env"),
        ("SLOPVAC_NO_UPDATE_CHECK", "env"),
        ("CI", "ci"),
        ("SLOPVAC_OFFLINE", "offline"),
    ] {
        let (_, doc) = w.json(&[], |c| {
            c.env(var, "1");
        });
        assert_eq!(doc["update"]["checks"], "disabled", "{var}");
        assert_eq!(doc["update"]["disabled_by"], why, "{var}");
    }
    // Offline wins over CI: it is the stronger, sticky reason.
    let (_, doc) = w.json(&[], |c| {
        c.env("CI", "1").env("SLOPVAC_OFFLINE", "1");
    });
    assert_eq!(doc["update"]["disabled_by"], "offline");
}

#[test]
fn the_lockfile_lists_each_pack_with_its_state() {
    let w = World::new();
    w.write(".slopvac/packs.lock", LOCK);
    let (code, doc) = w.json(&[], |c| {
        c.env("XDG_DATA_HOME", w.root.path().join("data"));
        c.env("XDG_STATE_HOME", w.root.path().join("state"));
    });
    assert_eq!(code, 0);
    assert_eq!(
        doc["packs"]["lockfile"],
        json!({"file": ".slopvac/packs.lock", "status": "ok", "error": null})
    );
    let items = doc["packs"]["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|p| p["id"].as_str().unwrap()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "sorted by id");
    assert_eq!(ids.len(), 4);
    let tagger = items.iter().find(|p| p["id"] == "acme-tagger").unwrap();
    assert_eq!(
        *tagger,
        json!({"id": "acme-tagger", "version": "0.3.1", "kind": "plugin", "state": "not-installed"})
    );
}

#[test]
fn an_invalid_lockfile_fails_the_check() {
    let w = World::new();
    w.write(
        ".slopvac/packs.lock",
        "schema_version = 3\n[[packs]]\nid = \"BAD ID\"\n",
    );
    let (code, doc) = w.json(&[], |_| {});
    assert_eq!(code, 2);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["packs"]["lockfile"]["status"], "invalid");
    assert_eq!(doc["packs"]["items"], json!([]));
    assert!(doc["packs"]["lockfile"]["error"].as_str().is_some());
}

#[test]
fn trust_reports_counts_and_fingerprints_but_never_keys() {
    use slopvac_plugins::trust::{DeclSummary, TrustRecord, TrustStore};
    let w = World::new();
    let state = w.root.path().join("state");
    let store = TrustStore::open(state.join("slopvac"));
    let key = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64();
    store.pin_key("acme", &key, "2026-11-15T00:00:00Z").unwrap();
    store
        .record(TrustRecord {
            hash: "sha256:".to_owned() + &"a".repeat(64),
            name: "acme-tagger".into(),
            version: Some("0.3.1".into()),
            source: "oci:ghcr.io/acme/p@sha256:x".into(),
            key_fingerprint: None,
            manifest_digest: None,
            key_id: None,
            declaration: DeclSummary {
                network: false,
                hooks: vec![],
                required_features: vec![],
                optional_features: vec![],
            },
            recorded_at: "2026-11-15T00:00:00Z".into(),
        })
        .unwrap();
    let (code, doc) = w.json(&[], |c| {
        c.env("XDG_STATE_HOME", &state);
    });
    assert_eq!(code, 0);
    assert_eq!(doc["trust"]["status"], "ok");
    assert_eq!(doc["trust"]["trusted_plugins"], 1);
    let pin = &doc["trust"]["pinned_keys"][0];
    assert_eq!(pin["namespace"], "acme");
    assert_eq!(pin["fingerprint"].as_str().unwrap().len(), 64);
    assert!(
        !doc.to_string().contains(&key),
        "the key itself was printed"
    );
}

#[test]
fn a_damaged_trust_store_fails_the_check() {
    let w = World::new();
    let state = w.root.path().join("state");
    fs::create_dir_all(state.join("slopvac")).unwrap();
    fs::write(state.join("slopvac/trust.json"), "{broken").unwrap();
    let (code, doc) = w.json(&[], |c| {
        c.env("XDG_STATE_HOME", &state);
    });
    assert_eq!(code, 2);
    assert_eq!(doc["trust"]["status"], "unreadable");
    assert_eq!(doc["ok"], false);
}

#[test]
fn directories_show_where_they_are_how_chosen_and_how_big() {
    let w = World::new();
    fs::write(w.cache().join("a"), vec![0u8; 100]).unwrap();
    fs::create_dir(w.cache().join("sub")).unwrap();
    fs::write(w.cache().join("sub/b"), vec![0u8; 23]).unwrap();
    let (_, doc) = w.json(&[], |_| {});
    let cache = &doc["directories"]["cache"];
    assert_eq!(cache["source"], "SLOPVAC_CACHE_DIR");
    assert_eq!(cache["exists"], true);
    assert_eq!(cache["bytes"], 123);
    assert_eq!(cache["truncated"], false);
    assert!(cache["path"].as_str().unwrap().starts_with("<external>/"));

    let data = &doc["directories"]["data"];
    assert!(cfg!(windows) || data["source"] == "default", "{data}");
    assert_eq!(data["exists"], false);
    assert_eq!(data["bytes"], Value::Null);

    let (_, doc) = w.json(&[], |c| {
        c.env("XDG_DATA_HOME", w.root.path().join("xdg-data"));
    });
    if !cfg!(windows) {
        assert_eq!(doc["directories"]["data"]["source"], "XDG_DATA_HOME");
    }
}

#[test]
fn without_a_home_the_directories_are_unresolved() {
    if cfg!(windows) {
        return;
    }
    let w = World::new();
    let (code, doc) = w.json(&[], |c| {
        c.env_remove("HOME").env_remove("SLOPVAC_CACHE_DIR");
    });
    assert_eq!(code, 0);
    for name in ["cache", "data", "state"] {
        let d = &doc["directories"][name];
        assert_eq!(d["path"], "<unresolved>", "{name}");
        assert_eq!(d["source"], "unresolved", "{name}");
        assert_eq!(d["exists"], false, "{name}");
    }
}

#[test]
fn doctor_changes_nothing_on_disk() {
    let w = World::new();
    w.write("slopvac.toml", "schema_version = 3\n");
    w.write(".slopvac/packs.lock", LOCK);
    let snapshot = |dir: &Path| -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(d).unwrap().flatten() {
                found.push(e.path());
                if e.path().is_dir() {
                    stack.push(e.path());
                }
            }
        }
        found.sort();
        found
    };
    let before = snapshot(w.root.path());
    w.json(&[], |_| {});
    w.doctor().assert().code(0);
    assert_eq!(snapshot(w.root.path()), before);
}
