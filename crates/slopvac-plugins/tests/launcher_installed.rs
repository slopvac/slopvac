//! A program that calls `launch::intercept` first thing runs
//! `Launcher::CurrentExe` for a plugin start and for a git-source sync. This
//! test binary has its own `main` so that it can be the launcher: the child
//! it starts is this binary again, started with the launcher flag.
//!
//! Without the registration in `intercept`, both checks fail with
//! `launcher-not-installed`.
#![cfg(unix)]

mod common;
mod oci_server;
mod revocations;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{Allow, cfg, spec};
use revocations::{ListServer, Root, Signed, list_json};
use slopvac_plugins::Plugin;
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::launch::Launcher;
use slopvac_plugins::lock::Lockfile;
use slopvac_plugins::oci::{Anonymous, Client};
use slopvac_plugins::revoke::Revocations;
use slopvac_plugins::sign::KeyPolicy;
use slopvac_plugins::sync::{Change, Network, Store, Syncer};

const LOCKED_HOST: &str = "https://git.test/";

/// The checks, by name.
const CHECKS: &[(&str, fn())] = &[
    (
        "plugin_starts_through_the_current_executable",
        plugin_starts_through_the_current_executable,
    ),
    (
        "git_source_syncs_through_the_current_executable",
        git_source_syncs_through_the_current_executable,
    ),
];

/// A minimal stand-in for the libtest harness, enough for `cargo test` and
/// nextest: `--list` names the checks, and the free arguments select which
/// run (all when none is given). A failed check panics and fails the binary.
fn main() {
    slopvac_plugins::launch::intercept();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            for (name, _) in CHECKS {
                println!("{name}: test");
            }
        }
        return;
    }
    let wanted: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    for (name, check) in CHECKS {
        if wanted.is_empty() || wanted.iter().any(|w| w == name) {
            check();
            println!("test {name} ... ok");
        }
    }
}

fn plugin_starts_through_the_current_executable() {
    let mut c = cfg();
    c.launcher = Launcher::CurrentExe;
    let p = Plugin::start(&spec("ok"), &c, &Allow).expect("plugin starts through CurrentExe");
    assert!(p.shutdown().clean());
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-11-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-01T00:00:00Z")
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn git_source_syncs_through_the_current_executable() {
    let files: &[(&str, &[u8])] = &[
        ("pack.toml", b"id = \"style\"\n"),
        ("rules/a.yml", b"rule: a\n"),
    ];
    let remote = tempfile::tempdir().unwrap();
    let work = remote.path().join("repo");
    fs::create_dir(&work).unwrap();
    git(&work, &["init", "--quiet"]);
    let plain = tempfile::tempdir().unwrap();
    for (name, data) in files {
        for root in [&work, plain.path()] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, data).unwrap();
        }
        git(&work, &["add", "--", name]);
    }
    git(&work, &["commit", "--quiet", "-m", "pack"]);
    let commit = git(&work, &["rev-parse", "HEAD"]);
    let digest = tree_hash(plain.path()).unwrap().to_string();
    let lock = Lockfile::parse(&format!(
        "schema_version = 3\n\n[[packs]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\n\
         slopvac_api = 1\nsource = \"git+{LOCKED_HOST}repo@{commit}\"\ndigest = \"{digest}\"\n",
    ))
    .unwrap();

    let root = Root::new();
    let empty = Signed::by(
        &root,
        list_json("2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z", &[], &[]),
    );
    let list = ListServer::start(Some(empty));
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let store = Store::new(data.path().join("slopvac"));
    let revocations = Revocations::new(
        data.path().join("state"),
        vec![root.public()],
        list.source(),
        "2026-11-14T00:00:00Z",
    )
    .unwrap();
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let policy = KeyPolicy::default();
    let served_from = format!("file://{}/", remote.path().display());

    let report = Syncer::new(
        &store,
        Network::Online(&client),
        &policy,
        project.path(),
        &revocations,
    )
    .with_launcher(Launcher::CurrentExe)
    .with_git_rewrite_for_tests(LOCKED_HOST, &served_from)
    .sync(&lock)
    .expect("git source syncs through CurrentExe");

    assert_eq!(report.packs[0].change, Change::Installed);
    assert_eq!(
        fs::read(report.packs[0].tree.join("rules/a.yml")).unwrap(),
        b"rule: a\n"
    );
}
