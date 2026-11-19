//! `slopvac self update` through the public `Runtime` interface, with an injected fetcher.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use slopvac_update::command::{self, Runtime};
use slopvac_update::fetch::{FetchError, Fetcher};
use slopvac_update::notice::Cache;
use slopvac_update::verify::{
    ArtifactVerifier, MinisignVerifier, ReleaseFile, ReleaseVerifier, VerifyError,
};

const LATEST: &str = "https://api.github.com/repos/slopvac/slopvac/releases/latest";
const BASE: &str = "https://example.test/dl";
/// The variable that locates the per-user config directory (receipts) on this platform.
const CONFIG_HOME_VAR: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else {
    "XDG_CONFIG_HOME"
};
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
    fn verify(&self, _: &[u8], _: &[u8], _: &ReleaseFile<'_>) -> Result<(), VerifyError> {
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
        "source": {"app_name": "slopvac", "owner": "slopvac", "name": "slopvac", "release_type": "github"},
    });
    std::fs::write(cfg.join("slopvac-receipt.json"), receipt.to_string()).unwrap();
    Fixture { dir, exe }
}

fn runtime(fx: &Fixture, server: Arc<Server>, verifier: Box<dyn ArtifactVerifier>) -> Runtime {
    let cfg = fx.dir.path().join("cfg").display().to_string();
    let cache = fx.dir.path().join("cache").display().to_string();
    let var = Box::new(move |k: &str| match k {
        k if k == CONFIG_HOME_VAR => Some(cfg.clone()),
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

/// A server whose archive carries `payload`, with `signature` as its `.minisig`.
fn signed_server(payload: &[u8], signature: &str) -> Arc<Server> {
    let url = format!("{BASE}/v3.1.0/slopvac-{TARGET}.tar.gz");
    Arc::new(Server {
        files: HashMap::from([
            (LATEST.to_owned(), br#"{"tag_name":"v3.1.0"}"#.to_vec()),
            (format!("{url}.minisig"), signature.as_bytes().to_vec()),
            (url, archive(payload)),
        ]),
        hits: AtomicUsize::new(0),
    })
}

/// Sign like release CI: prehashed, with `comment` as the trusted comment.
fn sign_with(keys: &minisign::KeyPair, bytes: &[u8], comment: Option<&str>) -> String {
    minisign::sign(None, &keys.sk, std::io::Cursor::new(bytes), comment, None)
        .unwrap()
        .to_string()
}

fn comment(target: &str, version: &str) -> String {
    ReleaseFile {
        name: &format!("slopvac-{target}.tar.gz"),
        version,
    }
    .trusted_comment()
}

/// The comment release CI writes for the archive this test requests.
fn sign(keys: &minisign::KeyPair, bytes: &[u8]) -> String {
    sign_with(keys, bytes, Some(&comment(TARGET, "3.1.0")))
}

/// Run `self update` against a verifier with `keys` and a server that serves `signature`.
fn update_with_signature(keys: &minisign::KeyPair, signature: &str) -> (Fixture, String) {
    let fx = installed();
    let verifier = MinisignVerifier::from_base64(&keys.pk.to_base64()).unwrap();
    let rt = runtime(
        &fx,
        signed_server(b"new binary", signature),
        Box::new(verifier),
    );
    let code = update(&rt).map_or_else(|f| f.code.to_owned(), |_| "ok".to_owned());
    (fx, code)
}

#[test]
fn archive_signed_by_the_verifier_key_is_installed() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let fx = installed();
    let sig = sign(&keys, &archive(b"new binary"));
    let verifier = MinisignVerifier::from_base64(&keys.pk.to_base64()).unwrap();
    let rt = runtime(&fx, signed_server(b"new binary", &sig), Box::new(verifier));
    update(&rt).unwrap();
    assert_eq!(content(&fx.exe), b"new binary");
}

#[test]
fn tampered_archive_fails_closed() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let fx = installed();
    // The signature covers the honest archive; the server delivers different bytes.
    let sig = sign(&keys, &archive(b"new binary"));
    let verifier = MinisignVerifier::from_base64(&keys.pk.to_base64()).unwrap();
    let rt = runtime(&fx, signed_server(b"evil binary", &sig), Box::new(verifier));
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn an_older_signed_release_served_at_the_new_url_fails_closed() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    // Genuine bytes, genuine key, but signed as the 3.0.9 release.
    let sig = sign_with(
        &keys,
        &archive(b"new binary"),
        Some(&comment(TARGET, "3.0.9")),
    );
    let (fx, code) = update_with_signature(&keys, &sig);
    assert_eq!(code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn another_targets_signed_archive_served_for_this_target_fails_closed() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let sig = sign_with(
        &keys,
        &archive(b"new binary"),
        Some(&comment("x86_64-unknown-linux-gnu", "3.1.0")),
    );
    let (fx, code) = update_with_signature(&keys, &sig);
    assert_eq!(code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn a_signature_without_the_release_trusted_comment_fails_closed() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let sig = sign_with(&keys, &archive(b"new binary"), None);
    let (fx, code) = update_with_signature(&keys, &sig);
    assert_eq!(code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn release_key_rejects_an_archive_signed_by_another_key() {
    let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let fx = installed();
    let sig = sign(&other, &archive(b"new binary"));
    let rt = runtime(
        &fx,
        signed_server(b"new binary", &sig),
        Box::new(ReleaseVerifier::default()),
    );
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "update-verification-failed");
    assert_eq!(content(&fx.exe), b"old binary");
}

#[test]
fn a_validly_signed_next_minor_candidate_is_refused_before_any_download() {
    const LINUX: &str = "x86_64-unknown-linux-gnu";
    let name = format!("slopvac-{LINUX}.tar.gz");
    let archive_bytes = include_bytes!("fixtures/slopvac-x86_64-unknown-linux-gnu.tar.gz");
    let rc_sig =
        include_bytes!("fixtures/slopvac-x86_64-unknown-linux-gnu.tar.gz.3.2.0-rc.1.minisig");
    let verifier = MinisignVerifier::from_pub_file(include_str!("fixtures/test-only.pub")).unwrap();
    // Precondition: the candidate is genuine, so only the stable-only rule can stop it.
    let rc = ReleaseFile {
        name: &name,
        version: "3.2.0-rc.1",
    };
    verifier
        .verify(archive_bytes, rc_sig, &rc)
        .expect("the fixture is a valid signed candidate");

    let url = format!("{BASE}/v3.2.0-rc.1/{name}");
    let srv = Arc::new(Server {
        files: HashMap::from([
            (LATEST.to_owned(), br#"{"tag_name":"v3.2.0-rc.1"}"#.to_vec()),
            (format!("{url}.minisig"), rc_sig.to_vec()),
            (url, archive_bytes.to_vec()),
        ]),
        hits: AtomicUsize::new(0),
    });
    let fx = installed();
    let mut rt = runtime(&fx, srv.clone(), Box::new(verifier));
    rt.target = Some(LINUX);
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "update-prerelease-refused");
    assert_eq!(
        srv.hits.load(Ordering::SeqCst),
        1,
        "only the version lookup"
    );
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
        err.message.contains("brew upgrade slopvac/tap/slopvac"),
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

#[test]
fn unknown_installs_point_to_the_slopvac_org_releases() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("somewhere/else/slopvac");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"old binary").unwrap();
    let fx = Fixture { dir, exe };
    let srv = server("3.1.0");
    let rt = runtime(&fx, srv.clone(), Box::new(Fixed(Ok(()))));
    let err = update(&rt).unwrap_err();
    assert_eq!(err.code, "self-update-unmanaged");
    assert!(
        err.message
            .contains("download a release from https://github.com/slopvac/slopvac/releases"),
        "{}",
        err.message
    );
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);
}

/// A release server for `version`, signed like release CI signs it.
fn signed_release(keys: &minisign::KeyPair, version: &str, binary: &[u8]) -> Server {
    let url = format!("{BASE}/v{version}/slopvac-{TARGET}.tar.gz");
    let bytes = archive(binary);
    let sig = sign_with(keys, &bytes, Some(&comment(TARGET, version)));
    Server {
        files: HashMap::from([
            (
                LATEST.to_owned(),
                format!(r#"{{"tag_name":"v{version}"}}"#).into_bytes(),
            ),
            (format!("{url}.minisig"), sig.into_bytes()),
            (url, bytes),
        ]),
        hits: AtomicUsize::new(0),
    }
}

/// Serves `inner`, and stops the first archive download until the test lets it go.
///
/// The download happens while the update holds the lock, so the test controls exactly when the
/// holder finishes without any sleep.
struct Gate {
    inner: Server,
    entered: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    held: std::sync::atomic::AtomicBool,
}

impl Fetcher for Gate {
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if url != LATEST && !self.held.swap(true, Ordering::SeqCst) {
            self.entered.lock().unwrap().send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        self.inner.get(url, limit)
    }
}

type Outcome = Result<String, command::Failure>;

/// Two `self update` runs on one installed binary. The first stops inside its download, so it
/// holds the lock. The second starts then and has to wait for the lock. Returns the install and
/// each run's outcome, first then second.
fn race(first: (&str, &[u8]), second: (&str, &[u8])) -> (Fixture, Outcome, Outcome) {
    use std::sync::mpsc::channel;

    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let fx = installed();
    let build = |version: &str, fetcher: Arc<dyn Fetcher + Send + Sync>| {
        let verifier = MinisignVerifier::from_base64(&keys.pk.to_base64()).unwrap();
        let mut rt = runtime(&fx, server(version), Box::new(verifier));
        rt.fetcher = fetcher;
        rt
    };
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (first_out, second_out) = std::thread::scope(|scope| {
        let first_run = scope.spawn(|| {
            // `Runtime` is not `Send`, so each thread builds its own.
            let gate = Gate {
                inner: signed_release(&keys, first.0, first.1),
                entered: std::sync::Mutex::new(entered_tx),
                release: std::sync::Mutex::new(release_rx),
                held: false.into(),
            };
            update(&build(first.0, Arc::new(gate)))
        });
        entered_rx.recv().unwrap();
        let second_run = scope.spawn(|| {
            let plain = signed_release(&keys, second.0, second.1);
            update(&build(second.0, Arc::new(plain)))
        });
        release_tx.send(()).unwrap();
        (first_run.join().unwrap(), second_run.join().unwrap())
    });
    (fx, first_out, second_out)
}

#[test]
fn racing_updates_end_on_the_newer_release_when_the_older_holds_the_lock_first() {
    let (fx, older, newer) = race(("3.1.0", b"binary 3.1.0"), ("3.2.0", b"binary 3.2.0"));
    assert_eq!(older.unwrap(), "Updated slopvac 3.0.0 to 3.1.0.\n");
    assert_eq!(newer.unwrap(), "Updated slopvac 3.0.0 to 3.2.0.\n");
    assert_eq!(content(&fx.exe), b"binary 3.2.0");
}

#[test]
fn a_waiting_older_update_does_not_roll_back_a_newer_install() {
    let (fx, newer, older) = race(("3.2.0", b"binary 3.2.0"), ("3.1.0", b"binary 3.1.0"));
    assert_eq!(newer.unwrap(), "Updated slopvac 3.0.0 to 3.2.0.\n");
    let refused = older.unwrap_err();
    assert_eq!(refused.code, "self-update-failed");
    assert!(
        refused
            .message
            .contains("refusing to install 3.1.0: it is not newer than the running 3.2.0"),
        "{}",
        refused.message
    );
    assert_eq!(content(&fx.exe), b"binary 3.2.0");
}

#[cfg(unix)]
#[test]
fn a_planted_lock_symlink_fails_with_its_own_code_and_leaves_the_target_alone() {
    let fx = installed();
    let victim = fx.dir.path().join("victim");
    std::fs::write(&victim, b"precious").unwrap();
    let lock = fx.exe.with_file_name("slopvac.update.lock");
    std::os::unix::fs::symlink(&victim, lock).unwrap();
    let rt = runtime(&fx, server("3.1.0"), Box::new(Fixed(Ok(()))));
    let failure = update(&rt).unwrap_err();
    assert_eq!(failure.code, "update-lock-unsafe");
    assert_eq!(content(&victim), b"precious");
    assert_eq!(content(&fx.exe), b"old binary");
}

/// Another installer puts a script at the install path that prints `body`. Returns the
/// fingerprint of the binary that this process "started" as.
#[cfg(unix)]
fn replaced_by_other_installer(fx: &Fixture, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let started_as = slopvac_update::pipeline::running_fingerprint(&fx.exe).unwrap();
    std::fs::write(&fx.exe, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&fx.exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    started_as
}

#[cfg(unix)]
#[test]
fn an_installed_binary_with_an_unknown_version_fails_with_its_own_code() {
    let fx = installed();
    let started_as = replaced_by_other_installer(&fx, "echo not json");
    let before = content(&fx.exe);
    let mut rt = runtime(&fx, server("3.2.0"), Box::new(Fixed(Ok(()))));
    rt.running_sha = Some(started_as);
    let failure = update(&rt).unwrap_err();
    assert_eq!(failure.code, "update-version-unknown");
    assert_eq!(content(&fx.exe), before);
}

/// Serves `inner`, and runs `during_lookup` when the latest version is asked for, which is the
/// first network call `self update` makes.
#[cfg(unix)]
struct SwapDuringLookup<F: Fn() + Send + Sync> {
    inner: Arc<Server>,
    during_lookup: F,
}

#[cfg(unix)]
impl<F: Fn() + Send + Sync> Fetcher for SwapDuringLookup<F> {
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if url == LATEST {
            (self.during_lookup)();
        }
        self.inner.get(url, limit)
    }
}

// The runtime takes no fingerprint up front here (`running_sha` is `None`, as in the real
// process). Another installer swaps in 3.3.0 while the update asks for the latest version. The
// update must not credit that new file to the running 3.0.0 and install 3.2.0 over it.
#[cfg(unix)]
#[test]
fn a_binary_swapped_during_the_version_lookup_is_not_taken_for_the_running_one() {
    use std::os::unix::fs::PermissionsExt;
    let fx = installed();
    let exe = fx.exe.clone();
    let mut rt = runtime(&fx, server("3.2.0"), Box::new(Fixed(Ok(()))));
    assert!(rt.running_sha.is_none());
    rt.fetcher = Arc::new(SwapDuringLookup {
        inner: server("3.2.0"),
        during_lookup: move || {
            std::fs::write(
                &exe,
                "#!/bin/sh\necho '{\"name\":\"slopvac\",\"version\":\"3.3.0\"}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        },
    });
    let failure = update(&rt).unwrap_err();
    assert_eq!(failure.code, "self-update-failed");
    assert!(
        failure
            .message
            .contains("refusing to install 3.2.0: it is not newer than the running 3.3.0"),
        "{}",
        failure.message
    );
    assert_ne!(content(&fx.exe), b"new binary");
}
