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

    /// The pack store root (`data` directory) the binary derives in this world.
    fn data_root(&self) -> PathBuf {
        if cfg!(windows) {
            self.home().join("slopvac").join("data")
        } else {
            self.root.path().join("xdg-data").join("slopvac")
        }
    }

    /// The directory that holds `trust.json` in this world.
    fn state_root(&self) -> PathBuf {
        if cfg!(windows) {
            self.home().join("slopvac").join("state")
        } else {
            self.root.path().join("xdg-state").join("slopvac")
        }
    }

    /// Point the data and state directories where `data_root` and `state_root` say. On Windows
    /// the binary derives them from `LOCALAPPDATA`, which `doctor()` already sets to the home.
    fn place_dirs(&self, cmd: &mut Command) {
        if !cfg!(windows) {
            cmd.env("XDG_DATA_HOME", self.root.path().join("xdg-data"))
                .env("XDG_STATE_HOME", self.root.path().join("xdg-state"));
        }
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
        self.doctor_with(Command::cargo_bin("slopvac").expect("binary builds"))
    }

    /// `doctor` on `cmd`, a command for some copy of the binary, with a clean environment.
    fn doctor_with(&self, mut cmd: Command) -> Command {
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
    const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3";
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
fn doctor_reports_an_unreadable_config_that_stops_every_other_command() {
    let w = World::new();
    fs::create_dir(w.project().join("slopvac.toml")).unwrap();
    // Doctor runs, prints the document and exits 2 itself.
    let (code, doc) = w.json(&[], |_| {});
    assert_eq!(code, 2);
    assert_eq!(doc["config"]["sources"][0]["status"], "unreadable");
    // Any other command stops on the same file before it runs.
    Command::cargo_bin("slopvac")
        .unwrap()
        .current_dir(w.project())
        .env("HOME", w.home())
        .args(["rules"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("error[config-unreadable]"));
    // And `doctor --help` is help, not a doctor run.
    Command::cargo_bin("slopvac")
        .unwrap()
        .current_dir(w.project())
        .args(["doctor", "--help"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[config-unreadable]"));
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
    let (code, doc) = w.json(&[], |c| w.place_dirs(c));
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
    let store = TrustStore::open(w.state_root());
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
            trust_key_id: None,
            declaration: DeclSummary {
                network: false,
                hooks: vec![],
                required_features: vec![],
                optional_features: vec![],
            },
            recorded_at: "2026-11-15T00:00:00Z".into(),
        })
        .unwrap();
    let (code, doc) = w.json(&[], |c| w.place_dirs(c));
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
    fs::create_dir_all(w.state_root()).unwrap();
    fs::write(w.state_root().join("trust.json"), "{broken").unwrap();
    let (code, doc) = w.json(&[], |c| w.place_dirs(c));
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

/// Both formats of a run, for leak checks.
fn both_formats(w: &World) -> (Value, String) {
    let (_, doc) = w.json(&[], |_| {});
    let text = String::from_utf8(w.doctor().output().unwrap().stdout).unwrap();
    (doc, text)
}

#[test]
fn lockfile_errors_say_where_and_never_echo_file_content() {
    const LEAK: &str = "LEAKME-4242";
    let bad_digest = format!(
        "schema_version = 3\n[[packs]]\nid = \"acme\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\nsource = \"oci://ghcr.io/acme/p:1\"\ndigest = \"sha256:{LEAK}\"\n"
    );
    let bad_id = format!("schema_version = 3\n[[packs]]\nid = \"{LEAK} BAD\"\n");
    let unknown_key = format!("schema_version = 3\n[[packs]]\n{LEAK} = 1\n");
    let syntax = format!("schema_version = 3\n{LEAK} = = =\n");
    let w = World::new();
    for (body, want) in [
        (bad_digest, "invalid value at /packs/0/digest"),
        (bad_id, "invalid value at /packs/0"),
        (unknown_key, "invalid value at /packs/0"),
        (syntax, "not a valid lockfile"),
    ] {
        w.write(".slopvac/packs.lock", &body);
        let (doc, text) = both_formats(&w);
        assert_eq!(doc["packs"]["lockfile"]["status"], "invalid", "{body}");
        assert_eq!(doc["packs"]["lockfile"]["error"], want, "{body}");
        assert!(!doc.to_string().contains(LEAK), "json leaked for {body}");
        assert!(!text.contains(LEAK), "text leaked for {body}");
        assert!(text.contains(want), "text: {text}");
    }
}

/// A pack installed in the store the way `plugins sync` lays it out: a tree named by its hash
/// and a ref from the locked digest to that tree.
fn install_pack(data_root: &Path, lock_digest_hex: &str) -> PathBuf {
    use slopvac_plugins::extract::tree_hash;
    use slopvac_plugins::sync::Store;
    let store = Store::new(data_root);
    for dir in ["trees", "blobs", "refs"] {
        fs::create_dir_all(data_root.join(dir)).unwrap();
    }
    let scratch = data_root.join("trees/scratch");
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("pack.toml"), "id = \"acme-docs\"\n").unwrap();
    let hash = tree_hash(&scratch).unwrap();
    let tree = store.tree_dir(&hash);
    fs::rename(&scratch, &tree).unwrap();
    fs::write(data_root.join("refs").join(lock_digest_hex), hash.hex()).unwrap();
    tree
}

const ACME_DOCS_DIGEST_HEX: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn an_intact_installed_pack_is_recorded_and_a_tampered_one_fails_the_check() {
    let w = World::new();
    w.write(".slopvac/packs.lock", LOCK);
    let tree = install_pack(&w.data_root(), ACME_DOCS_DIGEST_HEX);
    let with_data = |c: &mut Command| w.place_dirs(c);
    let state_of = |doc: &Value, id: &str| -> String {
        doc["packs"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()["state"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    let (code, doc) = w.json(&[], with_data);
    assert_eq!(code, 0);
    assert_eq!(state_of(&doc, "acme-docs"), "recorded");

    fs::write(tree.join("pack.toml"), "id = \"evil\"\n").unwrap();
    let (code, doc) = w.json(&[], with_data);
    assert_eq!(code, 2);
    assert_eq!(doc["ok"], false);
    assert_eq!(state_of(&doc, "acme-docs"), "tampered");
    // Doctor repairs nothing: the edited file is still there.
    assert_eq!(
        fs::read_to_string(tree.join("pack.toml")).unwrap(),
        "id = \"evil\"\n"
    );

    let mut cmd = w.doctor();
    with_data(&mut cmd);
    cmd.assert()
        .code(2)
        .stdout(predicate::str::contains("acme-docs 1.2.0 (data) tampered"))
        .stdout(predicate::str::ends_with("problems found\n"));
}

#[cfg(unix)]
#[test]
fn an_unreadable_trust_store_is_reported_not_called_absent() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        return; // root ignores permission bits
    }
    let w = World::new();
    let dir = w.state_root();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("trust.json"), "{}").unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let result = w.json(&[], |c| w.place_dirs(c));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let (code, doc) = result;
    assert_eq!(code, 2);
    assert_eq!(doc["trust"]["status"], "unreadable");
    assert_eq!(doc["ok"], false);
}

#[cfg(unix)]
#[test]
fn a_fifo_trust_file_does_not_hang_doctor() {
    let w = World::new();
    fs::create_dir_all(w.state_root()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(w.state_root().join("trust.json"))
            .status()
            .unwrap()
            .success()
    );
    let (code, doc) = w.json(&[], |c| {
        w.place_dirs(c);
        c.timeout(std::time::Duration::from_secs(20));
    });
    assert_eq!(code, 2);
    assert_eq!(doc["trust"]["status"], "unreadable");
}

/// Write a hand-made `trust.json` whose single pin is `pin`.
fn write_trust(w: &World, pin: &Value) {
    fs::create_dir_all(w.state_root()).unwrap();
    let doc = json!({"schema_version": 1, "records": [], "pins": [pin]});
    fs::write(w.state_root().join("trust.json"), doc.to_string()).unwrap();
}

fn pin(namespace: &str, fingerprint: &str, key: &str) -> Value {
    json!({
        "namespace": namespace,
        "fingerprint": fingerprint,
        "key": key,
        "pinned_at": "2026-11-15T00:00:00Z",
    })
}

#[test]
fn stored_trust_text_is_validated_before_it_is_printed() {
    const LEAK: &str = "LEAKME-9191";
    let good = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk;
    let good_key = good.to_base64();
    let other = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk;
    // The fingerprint of `good`, as the trust store derives it.
    let real = slopvac_plugins::sign::PublicKey::from_base64(&good_key)
        .unwrap()
        .fingerprint();
    let wrong = slopvac_plugins::sign::PublicKey::from_base64(&other.to_base64())
        .unwrap()
        .fingerprint();
    let cases = [
        ("an unparsable key", pin("acme", &real, LEAK)),
        ("an altered fingerprint", pin("acme", &wrong, &good_key)),
        (
            "a fingerprint that is not hex",
            pin("acme", LEAK, &good_key),
        ),
        (
            "a namespace with a path",
            pin(&format!("../../{LEAK}"), &real, &good_key),
        ),
        (
            "an uppercase namespace",
            pin(&format!("Acme-{LEAK}"), &real, &good_key),
        ),
        (
            "an overlong namespace",
            pin(&"a".repeat(65), &real, &good_key),
        ),
    ];
    for (what, bad) in cases {
        let w = World::new();
        write_trust(&w, &bad);
        let (code, doc) = w.json(&[], |c| w.place_dirs(c));
        assert_eq!(code, 2, "{what}");
        assert_eq!(doc["ok"], false, "{what}");
        assert_eq!(doc["trust"]["status"], "unreadable", "{what}");
        assert_eq!(
            doc["trust"]["error"], "the trust store holds an invalid pinned key",
            "{what}"
        );
        assert_eq!(doc["trust"]["pinned_keys"], json!([]), "{what}");
        let text = String::from_utf8({
            let mut cmd = w.doctor();
            w.place_dirs(&mut cmd);
            cmd.output().unwrap().stdout
        })
        .unwrap();
        assert!(!doc.to_string().contains(LEAK), "{what}: json leaked");
        assert!(!text.contains(LEAK), "{what}: text leaked");
    }

    // The same store with a valid pin is accepted, and the printed fingerprint is the derived one.
    let w = World::new();
    write_trust(&w, &pin("acme", &real, &good_key));
    let (code, doc) = w.json(&[], |c| w.place_dirs(c));
    assert_eq!(code, 0);
    assert_eq!(
        doc["trust"]["pinned_keys"],
        json!([{"namespace": "acme", "fingerprint": real}])
    );
}

/// The directory with links resolved, as the OS reports the working directory. On Windows
/// `canonicalize` adds the `\\?\` prefix, which the working directory does not have.
fn resolved(path: &Path) -> PathBuf {
    let real = fs::canonicalize(path).unwrap();
    let text = real.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if cfg!(windows) => PathBuf::from(rest),
        _ => real,
    }
}

#[test]
// Native Windows is not supported in 3.0 (slopvac-8l4.18.8); its cwd canonicalisation differs.
#[cfg_attr(windows, ignore = "native Windows is a 3.x feature (slopvac-8l4.18.8)")]
fn a_directory_that_is_the_working_directory_is_shown_as_a_dot() {
    let w = World::new();
    let (code, doc) = w.json(&[], |c| {
        c.env("SLOPVAC_CACHE_DIR", resolved(&w.project()));
    });
    assert_eq!(code, 0);
    assert_eq!(doc["directories"]["cache"]["path"], ".");
    assert_eq!(doc["directories"]["cache"]["exists"], true);
}

#[test]
fn a_cached_version_that_is_not_semver_counts_as_unknown() {
    let w = World::new();
    for garbage in ["not-a-version", "99", "v3.0.0", ""] {
        cache_entry(&w, garbage);
        let (code, doc) = w.json(&[], |_| {});
        assert_eq!(code, 0, "{garbage:?}");
        assert_eq!(doc["update"]["state"], "unknown", "{garbage:?}");
        assert_eq!(doc["update"]["latest"], Value::Null, "{garbage:?}");
        assert_eq!(doc["update"]["checked_at"], Value::Null, "{garbage:?}");
    }
}

/// A cargo-dist install receipt for `slopvac` in `dir/slopvac-receipt.json`.
fn write_receipt(dir: &Path, prefix: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("slopvac-receipt.json"),
        json!({
            "binaries": ["slopvac"],
            "install_layout": "flat",
            "install_prefix": prefix,
            "provider": {"source": "cargo-dist", "version": "0.34.0"},
            "source": {"app_name": "slopvac", "name": "slopvac", "owner": "slopvac", "release_type": "github"},
            "version": "3.0.0",
        })
        .to_string(),
    )
    .unwrap();
}

/// The directory that holds the binary these tests run.
fn binary_dir() -> PathBuf {
    let exe = assert_cmd::cargo::cargo_bin("slopvac");
    fs::canonicalize(exe)
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn an_installer_receipt_names_the_install_method_and_its_upgrade_command() {
    let w = World::new();
    let receipts = w.root.path().join("receipts");
    write_receipt(&receipts, &binary_dir());
    let (code, doc) = w.json(&[], |c| {
        c.env("AXOUPDATER_CONFIG_PATH", &receipts);
    });
    assert_eq!(code, 0);
    assert_eq!(doc["slopvac"]["install_method"], "installer");
    assert_eq!(doc["slopvac"]["upgrade_command"], "slopvac self update");
}

#[test]
fn a_receipt_for_another_location_leaves_the_method_unknown() {
    let w = World::new();
    let receipts = w.root.path().join("receipts");
    write_receipt(&receipts, &w.root.path().join("elsewhere"));
    let (_, doc) = w.json(&[], |c| {
        c.env("AXOUPDATER_CONFIG_PATH", &receipts);
    });
    assert_eq!(doc["slopvac"]["install_method"], "unknown");
    assert_eq!(doc["slopvac"]["upgrade_command"], Value::Null);
}

#[test]
fn the_text_report_gives_the_upgrade_command_when_an_update_is_cached() {
    let w = World::new();
    let receipts = w.root.path().join("receipts");
    write_receipt(&receipts, &binary_dir());
    cache_entry(&w, "99.0.0");
    let out = w
        .doctor()
        .env("AXOUPDATER_CONFIG_PATH", &receipts)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("installer"), "{text}");
    assert!(text.contains("slopvac self update"), "{text}");
    assert!(text.contains("99.0.0"), "{text}");
}

#[test]
fn the_text_report_points_a_cargo_install_at_the_releases_page_when_an_update_is_cached() {
    let w = World::new();
    // A binary under `<CARGO_HOME>/bin` with no receipt is classed as a cargo install.
    let cargo_home = fs::canonicalize(w.root.path()).unwrap().join("cargo-home");
    let bin = cargo_home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(assert_cmd::cargo::cargo_bin("slopvac").file_name().unwrap());
    fs::copy(assert_cmd::cargo::cargo_bin("slopvac"), &exe).unwrap();
    cache_entry(&w, "99.0.0");
    let out = w
        .doctor_with(Command::new(&exe))
        .env("CARGO_HOME", &cargo_home)
        .env_remove("AXOUPDATER_CONFIG_PATH")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("(cargo)"), "{text}");
    assert!(
        text.contains(
            "update: 99.0.0 is available. See https://github.com/slopvac/slopvac/releases.\n"
        ),
        "{text}"
    );
}

#[test]
fn doctor_never_writes_the_update_cache() {
    let w = World::new();
    let (_, _) = w.json(&[], |_| {});
    assert!(!w.cache().join("update-check.json").exists());
}

/// Make a FIFO with no writer at `path`.
#[cfg(unix)]
fn fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn a_fifo_as_the_update_cache_does_not_hang_doctor() {
    let w = World::new();
    fifo(&w.cache().join("update-check.json"));
    let (code, doc) = w.json(&[], |c| {
        c.timeout(std::time::Duration::from_secs(20));
    });
    assert_eq!(code, 0);
    assert_eq!(doc["update"]["state"], "unknown");
    assert_eq!(doc["update"]["latest"], Value::Null);
}

#[cfg(unix)]
#[test]
fn a_symlink_to_an_endless_device_as_the_update_cache_is_not_read() {
    let w = World::new();
    std::os::unix::fs::symlink("/dev/zero", w.cache().join("update-check.json")).unwrap();
    let (code, doc) = w.json(&[], |c| {
        c.timeout(std::time::Duration::from_secs(20));
    });
    assert_eq!(code, 0);
    assert_eq!(doc["update"]["state"], "unknown");
}

#[test]
fn an_oversize_update_cache_is_reported_unknown() {
    let w = World::new();
    let padding = "x".repeat(70 * 1024);
    fs::write(
        w.cache().join("update-check.json"),
        json!({"checked_at": 1u64, "latest": "99.0.0", "pad": padding}).to_string(),
    )
    .unwrap();
    let (_, doc) = w.json(&[], |_| {});
    assert_eq!(doc["update"]["state"], "unknown");
}

#[cfg(unix)]
#[test]
fn a_fifo_install_receipt_does_not_hang_doctor() {
    let w = World::new();
    let receipts = w.root.path().join("receipts");
    fs::create_dir_all(&receipts).unwrap();
    fifo(&receipts.join("slopvac-receipt.json"));
    let (code, doc) = w.json(&[], |c| {
        c.env("AXOUPDATER_CONFIG_PATH", &receipts)
            .timeout(std::time::Duration::from_secs(20));
    });
    assert_eq!(code, 0);
    assert_eq!(doc["slopvac"]["install_method"], "unknown");
}

#[test]
fn an_oversize_install_receipt_is_ignored() {
    let w = World::new();
    let receipts = w.root.path().join("receipts");
    write_receipt(&receipts, &binary_dir());
    let path = receipts.join("slopvac-receipt.json");
    let mut text = fs::read_to_string(&path).unwrap();
    text.insert_str(
        text.len() - 1,
        &format!(",\"pad\":\"{}\"", "x".repeat(80 * 1024)),
    );
    fs::write(&path, text).unwrap();
    let (_, doc) = w.json(&[], |c| {
        c.env("AXOUPDATER_CONFIG_PATH", &receipts);
    });
    assert_eq!(doc["slopvac"]["install_method"], "unknown");
}
