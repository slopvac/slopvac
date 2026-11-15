//! `slopvac self update` through the public `Runtime` interface, with an injected fetcher.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use slopvac_update::command::{self, Runtime};
use slopvac_update::fetch::{FetchError, Fetcher};
use slopvac_update::notice::Cache;
use slopvac_update::verify::{ArtifactVerifier, ReleaseVerifier, VerifyError};

const LATEST: &str = "https://api.github.com/repos/srobroek/slopvac/releases/latest";
const BASE: &str = "https://example.test/dl";
const TARGET: &str = "aarch64-apple-darwin";

struct Server {
    files: HashMap<String, Vec<u8>>,
    hits: AtomicUsize,
}

impl Fetcher for Server {
    fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, FetchError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| FetchError::Network(format!("404 {url}")))
    }
}

struct Fixed(Result<(), VerifyError>);
impl ArtifactVerifier for Fixed {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<(), VerifyError> {
        self.0.clone()
    }
}

fn archive(binary: &[u8]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(binary.len() as u64);
    h.set_mode(0o755);
    h.set_cksum();
    b.append_data(&mut h, "slopvac-aarch64-apple-darwin/slopvac", binary)
        .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &b.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

fn server(latest: &str) -> Arc<Server> {
    let url = format!("{BASE}/v{latest}/slopvac-{TARGET}.tar.gz");
    Arc::new(Server {
        files: HashMap::from([
            (
                LATEST.to_owned(),
                format!(r#"{{"tag_name":"v{latest}"}}"#).into_bytes(),
            ),
            (format!("{url}.minisig"), b"sig".to_vec()),
            (url, archive(b"new binary")),
        ]),
        hits: AtomicUsize::new(0),
    })
}

struct Fixture {
    dir: tempfile::TempDir,
    exe: std::path::PathBuf,
}

/// An installer-managed install: a receipt that names the install prefix.
fn installed() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("home/.cargo");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    let exe = prefix.join("bin/slopvac");
    std::fs::write(&exe, b"old binary").unwrap();
    let cfg = dir.path().join("cfg/slopvac");
    std::fs::create_dir_all(&cfg).unwrap();
    let receipt = serde_json::json!({
        "install_prefix": prefix,
        "source": {"app_name": "slopvac", "owner": "srobroek", "name": "slopvac", "release_type": "github"},
    });
    std::fs::write(cfg.join("slopvac-receipt.json"), receipt.to_string()).unwrap();
    Fixture { dir, exe }
}

fn runtime(fx: &Fixture, server: Arc<Server>, verifier: Box<dyn ArtifactVerifier>) -> Runtime {
    let cfg = fx.dir.path().join("cfg").display().to_string();
    let cache = fx.dir.path().join("cache").display().to_string();
    let var = Box::new(move |k: &str| match k {
        "XDG_CONFIG_HOME" => Some(cfg.clone()),
        "SLOPVAC_CACHE_DIR" => Some(cache.clone()),
        _ => None,
    });
    let mut rt = Runtime::with_var(false, "3.0.0", None, var);
    rt.exe.clone_from(&fx.exe);
    rt.fetcher = server;
    rt.verifier = verifier;
    BASE.clone_into(&mut rt.download_base);
    rt.target = Some(TARGET);
    rt.now = 5_000;
    rt
}

fn update(rt: &Runtime) -> Result<String, command::Failure> {
    let matches = command::command().get_matches_from(["self", "update"]);
    command::run(&matches, rt)
}

fn content(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn installer_install_updates_after_verification() {
    let fx = installed();
    let rt = runtime(&fx, server("3.1.0"), Box::new(Fixed(Ok(()))));
    let out = update(&rt).unwrap();
    assert_eq!(out, "Updated slopvac 3.0.0 to 3.1.0.\n");
    assert_eq!(content(&fx.exe), b"new binary");
    // The answer is cached, so the next notice needs no request.
    assert_eq!(
        Cache::default_for(&*rt.var).unwrap().read().unwrap().latest,
        "3.1.0"
    );
}

#[test]
fn verifier_rejection_leaves_the_binary_untouched() {
    let fx = installed();
    let rt = runtime(
        &fx,
        server("3.1.0"),
        Box::new(Fixed(Err(VerifyError::Rejected))),
    );
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn real_downloads_fail_closed_until_a_release_key_is_embedded() {
    // STUB(slopvac-8l4.13.25): when the key lands, `ReleaseVerifier` verifies for real and this
    // test becomes a tampered-artifact test against the embedded key.
    let fx = installed();
    let rt = runtime(&fx, server("3.1.0"), Box::new(ReleaseVerifier));
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn already_current_downloads_nothing() {
    let fx = installed();
    let srv = server("3.0.0");
    let rt = runtime(&fx, srv.clone(), Box::new(Fixed(Ok(()))));
    assert_eq!(update(&rt).unwrap(), "slopvac 3.0.0 is up to date.\n");
    assert_eq!(
        srv.hits.load(Ordering::SeqCst),
        1,
        "only the version lookup"
    );
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn offline_makes_no_request() {
    let fx = installed();
    let srv = server("3.1.0");
    let mut rt = runtime(&fx, srv.clone(), Box::new(Fixed(Ok(()))));
    rt.offline = true;
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "offline-network-needed");
    assert_eq!(err.message, "offline: self update needs the network");
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn other_installs_print_their_upgrade_command_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir
        .path()
        .join("opt/homebrew/Cellar/slopvac/3.0.0/bin/slopvac");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"old binary").unwrap();
    let fx = Fixture { dir, exe };
    let srv = server("3.1.0");
    let rt = runtime(&fx, srv.clone(), Box::new(Fixed(Ok(()))));
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "self-update-unmanaged");
    assert!(err.message.contains("homebrew"), "{}", err.message);
    assert!(
        err.message.contains("brew upgrade srobroek/tap/slopvac"),
        "{}",
        err.message
    );
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn version_json_reports_method_and_a_cached_notice() {
    let fx = installed();
    let rt = runtime(&fx, server("3.1.0"), Box::new(Fixed(Ok(()))));
    Cache::default_for(&*rt.var)
        .unwrap()
        .write(&slopvac_update::notice::Entry {
            checked_at: 5_000,
            latest: "4.0.0".into(),
        })
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&command::version_json(&rt)).unwrap();
    assert_eq!(doc["name"], "slopvac");
    assert_eq!(doc["version"], "3.0.0");
    assert_eq!(doc["install_method"], "installer");
    let n = &doc["notices"][0];
    assert_eq!(n["code"], "update-available");
    assert_eq!(n["x-latest"], "4.0.0");
    assert_eq!(n["x-upgrade-command"], "slopvac self update");
    assert!(
        n["x-migrate-hint"]
            .as_str()
            .unwrap()
            .contains("slopvac migrate")
    );
}

#[test]
fn version_json_never_starts_a_refresh() {
    let fx = installed();
    let srv = server("3.1.0");
    let rt = runtime(&fx, srv.clone(), Box::new(Fixed(Ok(()))));
    let doc: serde_json::Value = serde_json::from_str(&command::version_json(&rt)).unwrap();
    assert_eq!(doc["notices"], serde_json::json!([]));
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);
}
