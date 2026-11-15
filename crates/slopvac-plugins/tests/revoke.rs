//! Revocation list, its cache, the spawn-time check and key rotation
//! (threat model section 5.3, K5, K6; TM-26 spawn and status parts, TM-27).
//!
//! The fixed vectors in `fixtures/revoke` come from the `minisign` command line
//! and Python; see the README there. Scenario tests sign at run time with the
//! `minisign` crate and write the JSON by hand.

mod oci_server;
mod revocations;

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use revocations::{ListServer, Root, Signed, list_json};
use slopvac_plugins::extract::{Limits, extract_targz};
use slopvac_plugins::oci::{Anonymous, Client, Descriptor, Digest, Manifest, Reference};
use slopvac_plugins::protocol::{FeatureSet, PluginDeclaration};
use slopvac_plugins::revoke::{
    ARTIFACT_TYPE_REVOCATIONS, MEDIA_REVOCATIONS, RevocationError, RevocationList,
    RevocationNotice, Revocations, Revoked,
};
use slopvac_plugins::sign::{MEDIA_MINISIG, PublicKey, RotationProof, SignError, verify_rotation};
use slopvac_plugins::trust::{TrustError, TrustGate, TrustState, TrustStore, VerifiedPlugin};
use slopvac_plugins::{PluginError, PluginSpec, SpawnGate};

const ROOT_PUB: &str = include_str!("fixtures/revoke/root.pub");
const OLD_PUB: &str = include_str!("fixtures/revoke/old.pub");
const NEW_PUB: &str = include_str!("fixtures/revoke/new.pub");
const LIST: &[u8] = include_bytes!("fixtures/revoke/list.json");
const LIST_SIG: &str = include_str!("fixtures/revoke/list.minisig");
const LIST_FORGED_SIG: &str = include_str!("fixtures/revoke/list.forged.minisig");
const LIST_LEGACY_SIG: &str = include_str!("fixtures/revoke/list.legacy.minisig");
const ROTATION: &[u8] = include_bytes!("fixtures/revoke/rotation.json");
const ROTATION_OLD_SIG: &str = include_str!("fixtures/revoke/rotation.old.minisig");
const ROTATION_NEW_SIG: &str = include_str!("fixtures/revoke/rotation.new.minisig");

/// From the fixture README: sha256 of `revoked manifest`, and the key ids.
const REVOKED_DIGEST: &str =
    "sha256:9bf2927e4aa4f4343f08fb8878cfd0d4385febc236e2579d78031eff23f004d2";
const OLD_KEY_ID: &str = "C0E5E326B2F9B470";
const NEW_KEY_ID: &str = "A7628DAD436BB320";

/// The default list of the scenario tests: issued 2026-10-30, valid 30 days.
const ISSUED: &str = "2026-10-30T00:00:00Z";
const EXPIRES: &str = "2026-11-29T00:00:00Z";
const FETCHED: &str = "2026-11-01T00:00:00Z";

fn key(file: &str) -> PublicKey {
    PublicKey::from_base64(file.lines().nth(1).unwrap()).unwrap()
}

fn client() -> Client {
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
}

fn offline_source() -> Reference {
    Reference::parse("127.0.0.1:9/slopvac/revocations:latest").unwrap()
}

fn codes(notices: &[RevocationNotice]) -> Vec<&'static str> {
    notices.iter().map(RevocationNotice::code).collect()
}

// ---- The list format (fixed vectors) ----

#[test]
fn a_list_signed_by_the_root_names_its_keys_and_digests() {
    let list = RevocationList::verify(LIST, LIST_SIG, &[key(ROOT_PUB)]).unwrap();
    assert_eq!(list.issued(), Some("2026-11-12T00:00:00Z"));
    assert_eq!(list.expires(), Some("2026-12-12T00:00:00Z"));
    assert!(list.revokes_key(OLD_KEY_ID));
    assert!(list.revokes_digest(REVOKED_DIGEST));
    assert!(!list.revokes_key(NEW_KEY_ID));
    assert!(!list.revokes_digest(&Digest::of(b"another manifest").to_string()));
}

#[test]
fn tm27_a_list_signed_by_another_key_is_rejected() {
    let err = RevocationList::verify(LIST, LIST_FORGED_SIG, &[key(ROOT_PUB)]).unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");
    assert_eq!(err.code(), "revocation-list-invalid");
}

#[test]
fn tm27_an_edited_list_fails_under_the_root_signature() {
    // Drop the revoked digest: the bytes stay canonical, the signature does not cover them.
    let edited = String::from_utf8(LIST.to_vec())
        .unwrap()
        .replace(&format!("\"{REVOKED_DIGEST}\""), "");
    let err = RevocationList::verify(edited.as_bytes(), LIST_SIG, &[key(ROOT_PUB)]).unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");
}

#[test]
fn a_legacy_signature_is_refused() {
    let err = RevocationList::verify(LIST, LIST_LEGACY_SIG, &[key(ROOT_PUB)]).unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");
}

#[test]
fn with_no_root_key_every_list_fails_closed() {
    let err = RevocationList::verify(LIST, LIST_SIG, &[]).unwrap_err();
    assert!(matches!(err, RevocationError::NoRootKey), "{err:?}");
}

#[test]
fn a_publisher_key_cannot_sign_a_list() {
    // The old key is a valid minisign key, but not a root.
    let err = RevocationList::verify(LIST, LIST_SIG, &[key(OLD_PUB)]).unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");
}

#[test]
fn malformed_lists_are_refused_even_when_signed_by_the_root() {
    let root = Root::new();
    let mut ds = [Digest::of(b"1").to_string(), Digest::of(b"2").to_string()];
    ds.sort();
    let [lo, hi] = &ds;
    let mut ks = [OLD_KEY_ID, NEW_KEY_ID];
    ks.sort_unstable();
    let i = "2026-11-12T00:00:00Z";
    let e = "2026-12-12T00:00:00Z";
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("pretty printed", format!("{{ \"digests\": [], \"expires\": \"{e}\", \"issued\": \"{i}\", \"keys\": [], \"schema_version\": 1, \"type\": \"revocations\" }}").into_bytes()),
        ("unknown field", format!(r#"{{"digests":[],"expires":"{e}","extra":1,"issued":"{i}","keys":[],"schema_version":1,"type":"revocations"}}"#).into_bytes()),
        ("duplicate key", format!(r#"{{"digests":[],"digests":[],"expires":"{e}","issued":"{i}","keys":[],"schema_version":1,"type":"revocations"}}"#).into_bytes()),
        ("no expiry", format!(r#"{{"digests":[],"issued":"{i}","keys":[],"schema_version":1,"type":"revocations"}}"#).into_bytes()),
        ("wrong type", format!(r#"{{"digests":[],"expires":"{e}","issued":"{i}","keys":[],"schema_version":1,"type":"rotation"}}"#).into_bytes()),
        ("schema 2", format!(r#"{{"digests":[],"expires":"{e}","issued":"{i}","keys":[],"schema_version":2,"type":"revocations"}}"#).into_bytes()),
        ("bad issued time", list_json("2026-02-30T00:00:00Z", e, &[], &[])),
        ("bad expiry time", list_json(i, "2026-12-32T00:00:00Z", &[], &[])),
        ("expires when issued", list_json(i, i, &[], &[])),
        ("expires before issued", list_json(i, "2026-11-11T00:00:00Z", &[], &[])),
        ("valid for over 30 days", list_json(i, "2026-12-12T00:00:01Z", &[], &[])),
        ("lowercase key id", list_json(i, e, &[&OLD_KEY_ID.to_ascii_lowercase()], &[])),
        ("short key id", list_json(i, e, &[&OLD_KEY_ID[..8]], &[])),
        ("not a digest", list_json(i, e, &[], &["sha256:abc"])),
        ("unsorted digests", list_json(i, e, &[], &[hi, lo])),
        ("repeated digest", list_json(i, e, &[], &[lo, lo])),
        ("unsorted keys", list_json(i, e, &[ks[1], ks[0]], &[])),
    ];
    for (what, bytes) in cases {
        let signed = Signed::by(&root, bytes);
        let err = RevocationList::verify(&signed.statement, &signed.minisig, &[root.public()])
            .unwrap_err();
        assert!(
            matches!(err, RevocationError::Invalid(_)),
            "{what}: {err:?}"
        );
    }
}

#[test]
fn a_canonical_list_over_one_mib_is_refused_for_its_size() {
    let root = Root::new();
    // 15,000 sorted digests of 74 bytes each (quotes and comma) is over 1 MiB.
    let mut digests: Vec<String> = (0..15_000u32)
        .map(|n| Digest::of(&n.to_le_bytes()).to_string())
        .collect();
    digests.sort();
    let refs: Vec<&str> = digests.iter().map(String::as_str).collect();
    let bytes = list_json("2026-11-12T00:00:00Z", "2026-12-12T00:00:00Z", &[], &refs);
    assert!(bytes.len() > 1 << 20);
    let signed = Signed::by(&root, bytes);
    let err =
        RevocationList::verify(&signed.statement, &signed.minisig, &[root.public()]).unwrap_err();
    assert!(matches!(err, RevocationError::TooLarge), "{err:?}");
}

// ---- Cache and freshness (TM-27) ----

struct Env {
    root: Root,
    state: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            root: Root::new(),
            state: tempfile::tempdir().unwrap(),
        }
    }
    fn at(&self, source: Reference, now: &str) -> Revocations {
        Revocations::new(self.state.path(), vec![self.root.public()], source, now).unwrap()
    }
    fn offline(&self, now: &str) -> Revocations {
        self.at(offline_source(), now)
    }
    fn list(&self, issued: &str, expires: &str, keys: &[&str], digests: &[&str]) -> Signed {
        Signed::by(&self.root, list_json(issued, expires, keys, digests))
    }
    /// Fetch `signed` online at `now`, so the cache holds it.
    fn warm_at(&self, signed: Signed, now: &str) -> ListServer {
        let server = ListServer::start(Some(signed));
        self.at(server.source(), now).refresh(&client()).unwrap();
        server
    }
    fn warm(&self, signed: Signed) -> ListServer {
        self.warm_at(signed, FETCHED)
    }
}

#[test]
fn refresh_verifies_and_caches_the_list_for_offline_use() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[OLD_KEY_ID], &[&digest]));
    assert_eq!(server.fetches(), 1);

    let current = env.offline("2026-11-02T00:00:00Z").cached().unwrap();
    assert!(current.list.revokes_key(OLD_KEY_ID));
    assert!(current.list.revokes_digest(&digest));
    assert_eq!(current.fetched_at.as_deref(), Some(FETCHED));
    assert_eq!(current.notices, vec![]);
}

#[test]
fn tm27_an_expired_cache_offline_prints_a_notice_and_still_answers() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    // Fetched one day before the list expires: freshness follows the signed
    // expiry, not the fetch time.
    env.warm_at(
        env.list(ISSUED, EXPIRES, &[], &[&digest]),
        "2026-11-28T00:00:00Z",
    );
    for (now, stale) in [
        ("2026-11-29T00:00:00Z", false),
        ("2026-11-29T00:00:01Z", true),
        ("2027-03-01T00:00:00Z", true),
    ] {
        let current = env.offline(now).cached().unwrap();
        assert!(current.list.revokes_digest(&digest), "{now}");
        let want: Vec<&str> = if stale {
            vec!["revocation-list-stale"]
        } else {
            vec![]
        };
        assert_eq!(codes(&current.notices), want, "{now}");
    }
}

#[test]
fn tm27_the_stale_notice_names_the_signed_issue_and_expiry_times() {
    let env = Env::new();
    env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    let current = env.offline("2026-12-12T06:00:00Z").cached().unwrap();
    assert_eq!(
        current.notices,
        vec![RevocationNotice::Stale {
            issued: ISSUED.into(),
            expires: EXPIRES.into(),
        }]
    );
}

#[test]
fn tm27_a_replayed_list_past_its_expiry_fails_online_and_keeps_the_cache() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[&digest]));
    // The registry keeps serving the same valid list after it expired.
    let err = env
        .at(server.source(), "2026-11-29T00:00:01Z")
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::Expired { .. }), "{err:?}");
    assert_eq!(err.code(), "revocation-list-expired");
    let current = env.offline("2026-11-29T00:00:01Z").cached().unwrap();
    assert_eq!(current.fetched_at.as_deref(), Some(FETCHED));
    assert!(current.list.revokes_digest(&digest));
    assert_eq!(codes(&current.notices), vec!["revocation-list-stale"]);
}

#[test]
fn a_replayed_list_within_its_validity_is_accepted_online() {
    let env = Env::new();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    env.at(server.source(), "2026-11-28T23:59:59Z")
        .refresh(&client())
        .unwrap();
    let current = env.offline("2026-11-29T00:00:00Z").cached().unwrap();
    assert_eq!(current.fetched_at.as_deref(), Some("2026-11-28T23:59:59Z"));
}

#[test]
fn a_machine_that_never_fetched_runs_with_a_missing_list_notice() {
    let env = Env::new();
    let current = env.offline(FETCHED).cached().unwrap();
    assert_eq!(current.list, RevocationList::default());
    assert_eq!(current.fetched_at, None);
    assert_eq!(codes(&current.notices), vec!["revocation-list-missing"]);
}

#[test]
fn tm27_a_forged_list_online_is_rejected_and_the_cache_keeps_the_good_one() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[&digest]));
    let forger = Root::new();
    server.serve(Some(Signed::by(
        &forger,
        list_json("2026-11-05T00:00:00Z", "2026-12-05T00:00:00Z", &[], &[]),
    )));
    let err = env
        .at(server.source(), "2026-11-06T00:00:00Z")
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");

    let current = env.offline("2026-11-06T00:00:00Z").cached().unwrap();
    assert!(current.list.revokes_digest(&digest));
    assert_eq!(current.fetched_at.as_deref(), Some(FETCHED));
}

#[test]
fn k6_an_older_list_than_the_cache_is_a_rollback() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[&digest]));
    server.serve(Some(env.list(
        "2026-10-20T00:00:00Z",
        "2026-11-19T00:00:00Z",
        &[],
        &[],
    )));
    let err = env
        .at(server.source(), "2026-11-06T00:00:00Z")
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::Rollback { .. }), "{err:?}");
    assert_eq!(err.code(), "revocation-list-rollback");
    let current = env.offline("2026-11-06T00:00:00Z").cached().unwrap();
    assert!(current.list.revokes_digest(&digest));
}

#[test]
fn k6_a_withheld_list_fails_the_refresh() {
    let env = Env::new();
    let server = ListServer::start(None);
    let err = env
        .at(server.source(), FETCHED)
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::Fetch(_)), "{err:?}");
    assert_eq!(err.code(), "revocation-list-unavailable");
}

#[test]
fn a_newer_list_replaces_the_cache() {
    let env = Env::new();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    server.serve(Some(env.list(
        "2026-11-20T00:00:00Z",
        "2026-12-20T00:00:00Z",
        &[OLD_KEY_ID],
        &[],
    )));
    env.at(server.source(), "2026-12-01T00:00:00Z")
        .refresh(&client())
        .unwrap();
    let current = env.offline("2026-12-10T00:00:00Z").cached().unwrap();
    assert!(current.list.revokes_key(OLD_KEY_ID));
    assert_eq!(current.fetched_at.as_deref(), Some("2026-12-01T00:00:00Z"));
    assert_eq!(current.notices, vec![]);
}

fn wrong_artifact_type(m: &mut Manifest) {
    m.artifact_type = Some("application/vnd.slopvac.pack.v1".into());
}
fn extra_layer(m: &mut Manifest) {
    m.layers
        .push(Descriptor::for_bytes("application/octet-stream", b"{}"));
}
fn no_signature_layer(m: &mut Manifest) {
    m.layers.retain(|l| l.media_type != MEDIA_MINISIG);
}
fn two_list_layers(m: &mut Manifest) {
    for l in &mut m.layers {
        l.media_type = MEDIA_REVOCATIONS.into();
    }
}
fn two_signature_layers(m: &mut Manifest) {
    for l in &mut m.layers {
        l.media_type = MEDIA_MINISIG.into();
    }
}

type Edit = fn(&mut Manifest);

#[test]
fn a_correctly_signed_list_in_a_malformed_artifact_is_refused_and_the_cache_kept() {
    assert_ne!(ARTIFACT_TYPE_REVOCATIONS, "application/vnd.slopvac.pack.v1");
    let edits: [(&str, Edit); 5] = [
        ("wrong artifact type", wrong_artifact_type),
        ("extra layer", extra_layer),
        ("no signature layer", no_signature_layer),
        ("two list layers", two_list_layers),
        ("two signature layers", two_signature_layers),
    ];
    for (what, edit) in edits {
        let env = Env::new();
        let digest = Digest::of(b"bad").to_string();
        let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[&digest]));
        server.serve_edited(
            env.list("2026-11-05T00:00:00Z", "2026-12-05T00:00:00Z", &[], &[]),
            edit,
        );
        let err = env
            .at(server.source(), "2026-11-06T00:00:00Z")
            .refresh(&client())
            .unwrap_err();
        assert!(matches!(err, RevocationError::Layout(_)), "{what}: {err:?}");
        let current = env.offline("2026-11-06T00:00:00Z").cached().unwrap();
        assert!(current.list.revokes_digest(&digest), "{what}");
        assert_eq!(current.fetched_at.as_deref(), Some(FETCHED), "{what}");
    }
}

#[test]
fn k6_a_refresh_compares_and_replaces_under_the_cache_lock() {
    // A refresh with an older list waits for the lock, then compares against
    // what is cached by then, not what was cached when it started.
    let env = Env::new();
    env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    let older = ListServer::start(Some(env.list(
        "2026-10-31T00:00:00Z",
        "2026-11-30T00:00:00Z",
        &[],
        &[],
    )));
    // The cache file another process would write for a newer list.
    let newer_state = tempfile::tempdir().unwrap();
    let newer = ListServer::start(Some(env.list(
        "2026-11-05T00:00:00Z",
        "2026-12-05T00:00:00Z",
        &[OLD_KEY_ID],
        &[],
    )));
    Revocations::new(
        newer_state.path(),
        vec![env.root.public()],
        newer.source(),
        "2026-11-06T00:00:00Z",
    )
    .unwrap()
    .refresh(&client())
    .unwrap();
    let newer_cache = fs::read(newer_state.path().join("revocations.json")).unwrap();

    let revs = env.at(older.source(), "2026-11-06T00:00:00Z");
    let held = revs.lock().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            tx.send(revs.refresh(&client())).unwrap();
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(400)).is_err(),
            "the refresh replaced the cache while the lock was held"
        );
        fs::write(revs.cache_file(), &newer_cache).unwrap();
        drop(held);
        let got = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(
            matches!(got, Err(RevocationError::Rollback { .. })),
            "{got:?}"
        );
    });
    let current = env.offline("2026-11-06T00:00:00Z").cached().unwrap();
    assert_eq!(current.list.issued(), Some("2026-11-05T00:00:00Z"));
}

fn edit_cache(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "cache does not contain {from}");
    fs::write(path, text.replace(from, to)).unwrap();
}

#[test]
fn a_tampered_cache_fails_closed_and_a_refresh_repairs_it() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list(ISSUED, EXPIRES, &[], &[&digest]));
    let revs = env.offline("2026-11-02T00:00:00Z");
    // Remove the revoked digest from the cached statement.
    edit_cache(&revs.cache_file(), &format!("\\\"{digest}\\\""), "");
    let err = revs.cached().unwrap_err();
    assert!(
        matches!(err, RevocationError::CacheInvalid { .. }),
        "{err:?}"
    );
    assert_eq!(err.code(), "revocation-cache-invalid");

    env.at(server.source(), "2026-11-02T00:00:00Z")
        .refresh(&client())
        .unwrap();
    assert!(revs.cached().unwrap().list.revokes_digest(&digest));
}

#[test]
fn a_cache_that_is_not_a_cache_file_fails_closed() {
    let env = Env::new();
    env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    let revs = env.offline("2026-11-02T00:00:00Z");
    for body in ["", "{}", "not json", "{\"schema_version\":1}"] {
        fs::write(revs.cache_file(), body).unwrap();
        assert!(
            matches!(revs.cached(), Err(RevocationError::CacheInvalid { .. })),
            "{body:?}"
        );
    }
}

#[test]
fn an_invalid_clock_is_refused() {
    let env = Env::new();
    let err = Revocations::new(env.state.path(), vec![], offline_source(), "yesterday")
        .err()
        .unwrap();
    assert!(matches!(err, RevocationError::Clock(_)), "{err:?}");
}

#[cfg(unix)]
#[test]
fn the_cache_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    env.warm(env.list(ISSUED, EXPIRES, &[], &[]));
    let file = env.offline(FETCHED).cache_file();
    let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let dir = fs::metadata(file.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir, 0o700);
}

// ---- Spawn check from the cache and status flags (TM-26) ----

fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, name, *data).unwrap();
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&tar.into_inner().unwrap()).unwrap();
    enc.finish().unwrap()
}

fn install(dir: &Path, manifest: &str, key_id: &str) -> VerifiedPlugin {
    let out = extract_targz(
        targz(&[("plugin.py", b"print('hi')\n")]).as_slice(),
        dir,
        &HashSet::new(),
        Limits::default(),
    )
    .unwrap();
    VerifiedPlugin {
        name: "fake".into(),
        version: Some("1.0.0".into()),
        source: "oci:ghcr.io/acme/fake".into(),
        root: out.root,
        tree: out.hash,
        manifest_digest: Some(manifest.into()),
        key_fingerprint: Some("f".repeat(64)),
        key_id: Some(key_id.into()),
    }
}

fn spec_for(vp: &VerifiedPlugin) -> PluginSpec {
    PluginSpec {
        name: vp.name.clone(),
        program: vp.root.join("plugin.py"),
        args: vec![],
        cwd: vp.root.clone(),
        declaration: PluginDeclaration {
            hooks: vec![],
            features: FeatureSet::default(),
            network: false,
        },
        path_env: std::env::join_paths([std::env::temp_dir()])
            .unwrap()
            .into_string()
            .unwrap(),
        env: vec![],
    }
}

/// The gate a run builds once: it reads the revocation cache at every check.
fn gate(env: &Env, vp: &VerifiedPlugin, now: &str) -> TrustGate {
    TrustGate::new(
        TrustStore::open(env.state.path().join("trust")),
        vec![],
        vec![vp.clone()],
    )
    .with_revocation_cache(env.offline(now))
}

/// `hashlib.sha256(b"plugin manifest")`.
const MANIFEST: &str = "sha256:342b51663477cd347b07bde625d035780b2d14e8f89b80e7bfc24ad9a28554e7";

#[test]
fn tm26_a_revocation_cached_after_the_gate_was_built_stops_the_same_gate() {
    let other = Digest::of(b"other").to_string();
    let cases: [(&str, &[&str], &[&str]); 2] = [
        ("digest", &[], &[MANIFEST]),
        ("key", &[NEW_KEY_ID], &[other.as_str()]),
    ];
    for (what, keys, digests) in cases {
        let env = Env::new();
        let trees = tempfile::tempdir().unwrap();
        let vp = install(trees.path(), MANIFEST, NEW_KEY_ID);
        let spec = spec_for(&vp);
        let gate = gate(&env, &vp, "2026-11-02T00:00:00Z");
        let record = gate.record_trust(&vp, &spec, FETCHED).unwrap();
        gate.check(&spec).unwrap();

        env.warm(env.list(ISSUED, EXPIRES, keys, digests));
        assert_eq!(
            gate.state(&vp, &spec).unwrap().1,
            TrustState::Revoked,
            "{what}"
        );
        assert!(
            matches!(gate.check(&spec), Err(PluginError::Untrusted(_))),
            "{what}"
        );

        let list = env.offline(FETCHED).cached().unwrap().list;
        let want = if what == "digest" {
            Revoked::Digest(MANIFEST.into())
        } else {
            Revoked::Key(NEW_KEY_ID.into())
        };
        assert_eq!(list.flag(&record), Some(want), "{what}");
    }
}

#[test]
fn tm26_a_list_that_names_other_things_lets_the_plugin_run_and_flags_nothing() {
    let env = Env::new();
    let trees = tempfile::tempdir().unwrap();
    let vp = install(trees.path(), MANIFEST, NEW_KEY_ID);
    let spec = spec_for(&vp);
    let other = Digest::of(b"other").to_string();
    env.warm(env.list(ISSUED, EXPIRES, &[OLD_KEY_ID], &[&other]));
    let gate = gate(&env, &vp, "2026-11-02T00:00:00Z");
    let record = gate.record_trust(&vp, &spec, FETCHED).unwrap();
    gate.check(&spec).unwrap();
    let list = env.offline(FETCHED).cached().unwrap().list;
    assert_eq!(list.flag(&record), None);
}

#[test]
fn tm27_an_expired_cache_still_revokes_a_trusted_plugin() {
    let env = Env::new();
    let trees = tempfile::tempdir().unwrap();
    let vp = install(trees.path(), MANIFEST, NEW_KEY_ID);
    let spec = spec_for(&vp);
    let gate = gate(&env, &vp, "2027-06-01T00:00:00Z");
    gate.record_trust(&vp, &spec, FETCHED).unwrap();
    gate.check(&spec).unwrap();
    env.warm(env.list(ISSUED, EXPIRES, &[], &[MANIFEST]));
    assert_eq!(gate.state(&vp, &spec).unwrap().1, TrustState::Revoked);
}

#[test]
fn a_cache_tampered_after_the_gate_was_built_refuses_the_spawn() {
    let env = Env::new();
    let trees = tempfile::tempdir().unwrap();
    let vp = install(trees.path(), MANIFEST, NEW_KEY_ID);
    let spec = spec_for(&vp);
    env.warm(env.list(ISSUED, EXPIRES, &[], &[MANIFEST]));
    let gate = gate(&env, &vp, FETCHED);
    gate.record_trust(&vp, &spec, FETCHED).unwrap();
    edit_cache(
        &env.offline(FETCHED).cache_file(),
        &format!("\\\"{MANIFEST}\\\""),
        "",
    );
    assert!(matches!(
        gate.state(&vp, &spec),
        Err(TrustError::Revocation(RevocationError::CacheInvalid { .. }))
    ));
    assert!(matches!(gate.check(&spec), Err(PluginError::Untrusted(_))));
}

// ---- Rotation (section 5.3) ----

fn proof<'a>(old_sig: &'a str, new_sig: &'a str, new_key: &'a PublicKey) -> RotationProof<'a> {
    RotationProof {
        statement: ROTATION,
        old_signature: old_sig,
        new_signature: new_sig,
        new_key,
    }
}

#[test]
fn a_rotation_signed_by_the_old_and_the_new_key_verifies() {
    let new = key(NEW_PUB);
    let st = verify_rotation(
        &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new),
        "acme",
        &key(OLD_PUB),
        None,
    )
    .unwrap();
    assert_eq!(st.old_key_id, OLD_KEY_ID);
    assert_eq!(st.new_key_id, NEW_KEY_ID);
    assert_eq!(st.new_key, NEW_PUB.lines().nth(1).unwrap());
    assert_eq!(st.not_before, "2026-11-10T00:00:00Z");
}

#[test]
fn a_rotation_missing_either_signature_is_refused() {
    let new = key(NEW_PUB);
    let old = key(OLD_PUB);
    for (what, old_sig, new_sig) in [
        ("old signature only", ROTATION_OLD_SIG, ROTATION_OLD_SIG),
        ("new signature only", ROTATION_NEW_SIG, ROTATION_NEW_SIG),
        ("signatures swapped", ROTATION_NEW_SIG, ROTATION_OLD_SIG),
    ] {
        let err = verify_rotation(&proof(old_sig, new_sig, &new), "acme", &old, None).unwrap_err();
        assert_eq!(err, SignError::BadSignature, "{what}");
    }
}

#[test]
fn a_rotation_for_another_namespace_or_key_is_refused() {
    let new = key(NEW_PUB);
    let old = key(OLD_PUB);
    let p = proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new);
    assert!(matches!(
        verify_rotation(&p, "other", &old, None),
        Err(SignError::Mismatch(_))
    ));
    // The pin holds a key the statement does not name as old.
    assert!(matches!(
        verify_rotation(&p, "acme", &key(ROOT_PUB), None),
        Err(SignError::KeyNotPinned { .. })
    ));
    // The offered new key is not the one the statement names.
    let wrong_new = key(ROOT_PUB);
    assert!(matches!(
        verify_rotation(
            &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &wrong_new),
            "acme",
            &old,
            None
        ),
        Err(SignError::Mismatch(_))
    ));
}

/// Re-label `raw` (algorithm, key id, key) with `id`, the 8 bytes at 2..10.
fn with_key_id(mut raw: Vec<u8>, id: &[u8]) -> Vec<u8> {
    raw[2..10].copy_from_slice(id);
    raw
}

#[test]
fn k5_a_different_key_that_claims_the_new_key_id_cannot_complete_a_rotation() {
    // A minisign key id is unsigned metadata: an attacker can give their own key
    // the new key's id and re-label their signature to match. The statement
    // binds the whole new key, so the substitute is refused.
    let genuine = STANDARD.decode(NEW_PUB.lines().nth(1).unwrap()).unwrap();
    let id = &genuine[2..10];
    let attacker = Root::new();
    let forged_key =
        PublicKey::from_base64(&STANDARD.encode(with_key_id(attacker.raw(), id))).unwrap();
    assert_eq!(forged_key.key_id(), NEW_KEY_ID);
    let mut lines: Vec<String> = attacker.sign(ROTATION).lines().map(str::to_owned).collect();
    lines[1] = STANDARD.encode(with_key_id(STANDARD.decode(&lines[1]).unwrap(), id));
    let forged_sig = lines.join("\n") + "\n";

    let err = verify_rotation(
        &proof(ROTATION_OLD_SIG, &forged_sig, &forged_key),
        "acme",
        &key(OLD_PUB),
        None,
    )
    .unwrap_err();
    assert!(matches!(err, SignError::Mismatch(_)), "{err:?}");
}

#[test]
fn a_rotation_must_be_newer_than_the_pins_not_before() {
    let new = key(NEW_PUB);
    let old = key(OLD_PUB);
    let p = proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new);
    for (floor, ok) in [
        ("2026-11-09T23:59:59Z", true),
        ("2026-11-10T00:00:00Z", false),
        ("2026-12-01T00:00:00Z", false),
    ] {
        let got = verify_rotation(&p, "acme", &old, Some(floor));
        if ok {
            got.unwrap();
        } else {
            assert!(
                matches!(got, Err(SignError::Rollback(_))),
                "{floor}: {got:?}"
            );
        }
    }
}

fn pinned_ids(store: &TrustStore) -> Vec<String> {
    store.pinned_map().unwrap()["acme"]
        .iter()
        .map(PublicKey::key_id)
        .collect()
}

#[test]
fn rotation_replaces_the_pin_and_sets_its_not_before() {
    let state = tempfile::tempdir().unwrap();
    let store = TrustStore::open(state.path());
    store
        .pin_key("acme", OLD_PUB.lines().nth(1).unwrap(), FETCHED)
        .unwrap();
    let new = key(NEW_PUB);
    let pin = store
        .rotate_key(
            "acme",
            &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new),
            &RevocationList::default(),
            "2026-11-14T00:00:00Z",
        )
        .unwrap();
    assert_eq!(pin.fingerprint, new.fingerprint());
    assert_eq!(pin.not_before.as_deref(), Some("2026-11-10T00:00:00Z"));
    assert_eq!(pinned_ids(&store), vec![NEW_KEY_ID.to_owned()]);

    // The same statement again no longer names the pinned key as the old one.
    let again = store.rotate_key(
        "acme",
        &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new),
        &RevocationList::default(),
        "2026-11-15T00:00:00Z",
    );
    assert!(
        matches!(
            again,
            Err(TrustError::Rotation(SignError::KeyNotPinned { .. }))
        ),
        "{again:?}"
    );
}

#[test]
fn rotation_needs_an_existing_pin() {
    let state = tempfile::tempdir().unwrap();
    let store = TrustStore::open(state.path());
    let new = key(NEW_PUB);
    let err = store
        .rotate_key(
            "acme",
            &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new),
            &RevocationList::default(),
            FETCHED,
        )
        .unwrap_err();
    assert!(matches!(err, TrustError::NotPinned(_)), "{err:?}");
    assert!(store.pinned_map().unwrap().is_empty());
}

#[test]
fn k5_a_revoked_old_key_cannot_rotate_to_a_new_one() {
    // The fixture list revokes the old key: whoever stole it cannot rotate away from it.
    let list = RevocationList::verify(LIST, LIST_SIG, &[key(ROOT_PUB)]).unwrap();
    let state = tempfile::tempdir().unwrap();
    let store = TrustStore::open(state.path());
    store
        .pin_key("acme", OLD_PUB.lines().nth(1).unwrap(), FETCHED)
        .unwrap();
    let new = key(NEW_PUB);
    let err = store
        .rotate_key(
            "acme",
            &proof(ROTATION_OLD_SIG, ROTATION_NEW_SIG, &new),
            &list,
            "2026-11-14T00:00:00Z",
        )
        .unwrap_err();
    assert!(matches!(err, TrustError::Revoked(_)), "{err:?}");
    assert_eq!(pinned_ids(&store), vec![OLD_KEY_ID.to_owned()]);
}

/// A rotation statement written by hand, with `kind` and `schema` free.
fn rotation_doc(
    old: &PublicKey,
    new: &PublicKey,
    not_before: &str,
    kind: &str,
    schema: u32,
) -> Vec<u8> {
    format!(
        "{{\"namespace\":\"acme\",\"newKey\":\"{}\",\"newKeyId\":\"{}\",\"notBefore\":\"{not_before}\",\"oldKeyId\":\"{}\",\"schema_version\":{schema},\"type\":\"{kind}\"}}",
        new.to_base64(),
        new.key_id(),
        old.key_id()
    )
    .into_bytes()
}

fn rotate_with(
    store: &TrustStore,
    old: &Root,
    new: &Root,
    statement: &[u8],
    revoked: &RevocationList,
) -> Result<(), TrustError> {
    let (old_sig, new_sig) = (old.sign(statement), new.sign(statement));
    let new_key = new.public();
    let p = RotationProof {
        statement,
        old_signature: &old_sig,
        new_signature: &new_sig,
        new_key: &new_key,
    };
    store
        .rotate_key("acme", &p, revoked, "2026-12-31T00:00:00Z")
        .map(drop)
}

fn rotate(store: &TrustStore, old: &Root, new: &Root, not_before: &str) -> Result<(), TrustError> {
    let statement = rotation_doc(&old.public(), &new.public(), not_before, "rotation", 1);
    rotate_with(store, old, new, &statement, &RevocationList::default())
}

fn pinned_store(key: &Root) -> (tempfile::TempDir, TrustStore) {
    let state = tempfile::tempdir().unwrap();
    let store = TrustStore::open(state.path());
    store
        .pin_key("acme", &key.public().to_base64(), FETCHED)
        .unwrap();
    (state, store)
}

#[test]
fn k5_a_rotation_to_a_revoked_key_is_refused_and_the_pin_kept() {
    let (a, b, root) = (Root::new(), Root::new(), Root::new());
    let (_state, store) = pinned_store(&a);
    let id = b.public().key_id();
    let signed = Signed::by(
        &root,
        list_json("2026-11-12T00:00:00Z", "2026-12-12T00:00:00Z", &[&id], &[]),
    );
    let list =
        RevocationList::verify(&signed.statement, &signed.minisig, &[root.public()]).unwrap();
    let statement = rotation_doc(
        &a.public(),
        &b.public(),
        "2026-11-10T00:00:00Z",
        "rotation",
        1,
    );
    let err = rotate_with(&store, &a, &b, &statement, &list).unwrap_err();
    assert!(matches!(err, TrustError::Revoked(_)), "{err:?}");
    assert_eq!(pinned_ids(&store), vec![a.public().key_id()]);
}

#[test]
fn a_signed_statement_that_is_not_a_version_1_rotation_is_refused_and_the_pin_kept() {
    let (a, b) = (Root::new(), Root::new());
    let (_state, store) = pinned_store(&a);
    for (kind, schema) in [("revocations", 1), ("rotation", 2), ("rotation", 0)] {
        let statement = rotation_doc(
            &a.public(),
            &b.public(),
            "2026-11-10T00:00:00Z",
            kind,
            schema,
        );
        let err = rotate_with(&store, &a, &b, &statement, &RevocationList::default()).unwrap_err();
        assert!(
            matches!(err, TrustError::Rotation(SignError::InvalidStatement(_))),
            "{kind} {schema}: {err:?}"
        );
        assert_eq!(pinned_ids(&store), vec![a.public().key_id()]);
    }
}

#[test]
fn a_second_rotation_must_be_newer_than_the_first() {
    let (a, b, c) = (Root::new(), Root::new(), Root::new());
    let (_state, store) = pinned_store(&a);
    rotate(&store, &a, &b, "2026-11-10T00:00:00Z").unwrap();

    for older in ["2026-11-10T00:00:00Z", "2026-11-01T00:00:00Z"] {
        let err = rotate(&store, &b, &c, older).unwrap_err();
        assert!(
            matches!(err, TrustError::Rotation(SignError::Rollback(_))),
            "{older}: {err:?}"
        );
    }
    rotate(&store, &b, &c, "2026-11-10T00:00:01Z").unwrap();
    assert_eq!(pinned_ids(&store), vec![c.public().key_id()]);
}
