//! Revocation list, its cache, the spawn-time check and key rotation
//! (threat model section 5.3, K5, K6; TM-26 spawn and status parts, TM-27).
//!
//! The fixed vectors in `fixtures/revoke` come from the `minisign` command line
//! and Python; see the README there. Scenario tests sign at run time with the
//! `minisign` crate and write the list JSON by hand.

mod oci_server;
mod revocations;

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use revocations::{ListServer, Root, Signed, list_json};
use slopvac_plugins::extract::{Limits, extract_targz};
use slopvac_plugins::oci::{Anonymous, Client, Digest, Reference};
use slopvac_plugins::protocol::{FeatureSet, PluginDeclaration};
use slopvac_plugins::revoke::{
    RevocationError, RevocationList, RevocationNotice, Revocations, Revoked,
};
use slopvac_plugins::sign::{PublicKey, RotationProof, SignError, verify_rotation};
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
const OLD_KEY_ID: &str = "8384841ACDC75F77";
const NEW_KEY_ID: &str = "7FF478DC0F88DED4";

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

fn sorted(mut v: Vec<&str>) -> Vec<&str> {
    v.sort_unstable();
    v
}

// ---- The list format (fixed vectors) ----

#[test]
fn a_list_signed_by_the_root_names_its_keys_and_digests() {
    let list = RevocationList::verify(LIST, LIST_SIG, &[key(ROOT_PUB)]).unwrap();
    assert_eq!(list.issued(), Some("2026-11-12T00:00:00Z"));
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
    let d1 = Digest::of(b"1").to_string();
    let d2 = Digest::of(b"2").to_string();
    let (lo, hi) = if d1 < d2 { (d1, d2) } else { (d2, d1) };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("pretty printed", b"{ \"digests\": [], \"issued\": \"2026-11-12T00:00:00Z\", \"keys\": [], \"schema_version\": 1, \"type\": \"revocations\" }".to_vec()),
        ("unknown field", br#"{"digests":[],"extra":1,"issued":"2026-11-12T00:00:00Z","keys":[],"schema_version":1,"type":"revocations"}"#.to_vec()),
        ("duplicate key", br#"{"digests":[],"digests":[],"issued":"2026-11-12T00:00:00Z","keys":[],"schema_version":1,"type":"revocations"}"#.to_vec()),
        ("wrong type", br#"{"digests":[],"issued":"2026-11-12T00:00:00Z","keys":[],"schema_version":1,"type":"rotation"}"#.to_vec()),
        ("schema 2", br#"{"digests":[],"issued":"2026-11-12T00:00:00Z","keys":[],"schema_version":2,"type":"revocations"}"#.to_vec()),
        ("bad time", list_json("2026-02-30T00:00:00Z", &[], &[])),
        ("lowercase key id", list_json("2026-11-12T00:00:00Z", &["8384841acdc75f77"], &[])),
        ("short key id", list_json("2026-11-12T00:00:00Z", &["8384841A"], &[])),
        ("not a digest", list_json("2026-11-12T00:00:00Z", &[], &["sha256:abc"])),
        ("unsorted digests", list_json("2026-11-12T00:00:00Z", &[], &[&hi, &lo])),
        ("repeated digest", list_json("2026-11-12T00:00:00Z", &[], &[&lo, &lo])),
        ("unsorted keys", list_json("2026-11-12T00:00:00Z", &[OLD_KEY_ID, NEW_KEY_ID], &[])),
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
fn a_list_over_one_mib_is_refused_before_parsing() {
    let root = Root::new();
    let mut bytes = list_json("2026-11-12T00:00:00Z", &[], &[]);
    bytes.resize((1 << 20) + 1, b' ');
    let signed = Signed::by(&root, bytes);
    let err =
        RevocationList::verify(&signed.statement, &signed.minisig, &[root.public()]).unwrap_err();
    assert!(matches!(err, RevocationError::Invalid(_)), "{err:?}");
}

// ---- Cache and staleness (TM-27) ----

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
    fn list(&self, issued: &str, keys: &[&str], digests: &[&str]) -> Signed {
        Signed::by(&self.root, list_json(issued, keys, digests))
    }
    /// Fetch `signed` online at `FETCHED`, so the cache holds it.
    fn warm(&self, signed: Signed) -> ListServer {
        let server = ListServer::start(Some(signed));
        self.at(server.source(), FETCHED)
            .refresh(&client())
            .unwrap();
        server
    }
}

#[test]
fn refresh_verifies_and_caches_the_list_for_offline_use() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list("2026-10-30T00:00:00Z", &[OLD_KEY_ID], &[&digest]));
    assert_eq!(server.fetches(), 1);

    let current = env
        .at(offline_source(), "2026-11-02T00:00:00Z")
        .cached()
        .unwrap();
    assert!(current.list.revokes_key(OLD_KEY_ID));
    assert!(current.list.revokes_digest(&digest));
    assert_eq!(current.fetched_at.as_deref(), Some(FETCHED));
    assert_eq!(current.notices, vec![]);
}

#[test]
fn tm27_a_stale_cache_prints_a_notice_and_still_answers() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&digest]));
    // 30 days after FETCHED is the last fresh second; one second later is stale.
    for (now, stale) in [
        ("2026-12-01T00:00:00Z", false),
        ("2026-12-01T00:00:01Z", true),
        ("2027-03-01T00:00:00Z", true),
    ] {
        let current = env.at(offline_source(), now).cached().unwrap();
        assert!(current.list.revokes_digest(&digest), "{now}");
        let notices: Vec<&str> = current.notices.iter().map(RevocationNotice::code).collect();
        let want: Vec<&str> = if stale {
            vec!["revocation-list-stale"]
        } else {
            vec![]
        };
        assert_eq!(notices, want, "{now}");
    }
}

#[test]
fn tm27_the_stale_notice_names_the_fetch_time_and_age_in_days() {
    let env = Env::new();
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[]));
    let current = env
        .at(offline_source(), "2026-12-12T06:00:00Z")
        .cached()
        .unwrap();
    assert_eq!(
        current.notices,
        vec![RevocationNotice::Stale {
            fetched_at: FETCHED.into(),
            days: 41,
        }]
    );
}

#[test]
fn a_machine_that_never_fetched_runs_with_a_missing_list_notice() {
    let env = Env::new();
    let current = env.at(offline_source(), FETCHED).cached().unwrap();
    assert_eq!(current.list, RevocationList::default());
    assert_eq!(current.fetched_at, None);
    let codes: Vec<&str> = current.notices.iter().map(RevocationNotice::code).collect();
    assert_eq!(codes, vec!["revocation-list-missing"]);
}

#[test]
fn tm27_a_forged_list_online_is_rejected_and_the_cache_keeps_the_good_one() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&digest]));
    let forger = Root::new();
    server.serve(Some(Signed::by(
        &forger,
        list_json("2026-11-05T00:00:00Z", &[], &[]),
    )));
    let err = env
        .at(server.source(), "2026-11-06T00:00:00Z")
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::BadSignature), "{err:?}");

    let current = env
        .at(offline_source(), "2026-11-06T00:00:00Z")
        .cached()
        .unwrap();
    assert!(current.list.revokes_digest(&digest));
    assert_eq!(current.fetched_at.as_deref(), Some(FETCHED));
}

#[test]
fn k6_an_older_list_than_the_cache_is_a_rollback() {
    let env = Env::new();
    let digest = Digest::of(b"bad").to_string();
    let server = env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&digest]));
    server.serve(Some(env.list("2026-10-01T00:00:00Z", &[], &[])));
    let err = env
        .at(server.source(), "2026-11-06T00:00:00Z")
        .refresh(&client())
        .unwrap_err();
    assert!(matches!(err, RevocationError::Rollback { .. }), "{err:?}");
    assert_eq!(err.code(), "revocation-list-rollback");
    let current = env
        .at(offline_source(), "2026-11-06T00:00:00Z")
        .cached()
        .unwrap();
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
fn a_newer_list_replaces_the_cache_and_resets_its_age() {
    let env = Env::new();
    let server = env.warm(env.list("2026-10-30T00:00:00Z", &[], &[]));
    server.serve(Some(env.list("2026-11-20T00:00:00Z", &[OLD_KEY_ID], &[])));
    env.at(server.source(), "2026-12-20T00:00:00Z")
        .refresh(&client())
        .unwrap();
    let current = env
        .at(offline_source(), "2027-01-10T00:00:00Z")
        .cached()
        .unwrap();
    assert!(current.list.revokes_key(OLD_KEY_ID));
    assert_eq!(current.fetched_at.as_deref(), Some("2026-12-20T00:00:00Z"));
    assert_eq!(current.notices, vec![]);
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
    let server = env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&digest]));
    let revs = env.at(offline_source(), "2026-11-02T00:00:00Z");
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
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[]));
    let revs = env.at(offline_source(), "2026-11-02T00:00:00Z");
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
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[]));
    let file = env.at(offline_source(), FETCHED).cache_file();
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

/// A trusted plugin and a gate built the way spawn builds it: from the cache.
fn gate_from_cache(
    env: &Env,
    vp: &VerifiedPlugin,
    now: &str,
) -> Result<TrustGate, RevocationError> {
    let current = env.at(offline_source(), now).cached()?;
    Ok(TrustGate::new(
        TrustStore::open(env.state.path().join("trust")),
        vec![],
        vec![vp.clone()],
    )
    .with_revoked(current.list.revocations()))
}

#[test]
fn tm26_a_revoked_key_or_digest_in_the_cache_refuses_the_spawn_and_flags_the_record() {
    let manifest = Digest::of(b"plugin manifest").to_string();
    let other = Digest::of(b"other").to_string();
    let cases = [
        ("digest", sorted(vec![]), sorted(vec![manifest.as_str()])),
        (
            "key",
            sorted(vec![NEW_KEY_ID]),
            sorted(vec![other.as_str()]),
        ),
    ];
    for (what, keys, digests) in cases {
        let env = Env::new();
        let trees = tempfile::tempdir().unwrap();
        let vp = install(trees.path(), &manifest, NEW_KEY_ID);
        let spec = spec_for(&vp);
        let clean = gate_from_cache(&env, &vp, FETCHED).unwrap();
        let record = clean.record_trust(&vp, &spec, FETCHED).unwrap();
        clean.check(&spec).unwrap();

        env.warm(env.list("2026-10-30T00:00:00Z", &keys, &digests));
        let gate = gate_from_cache(&env, &vp, "2026-11-02T00:00:00Z").unwrap();
        assert_eq!(
            gate.state(&vp, &spec).unwrap().1,
            TrustState::Revoked,
            "{what}"
        );
        assert!(
            matches!(gate.check(&spec), Err(PluginError::Untrusted(_))),
            "{what}"
        );

        let list = env.at(offline_source(), FETCHED).cached().unwrap().list;
        let want = if what == "digest" {
            Revoked::Digest(manifest.clone())
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
    let manifest = Digest::of(b"plugin manifest").to_string();
    let vp = install(trees.path(), &manifest, NEW_KEY_ID);
    let spec = spec_for(&vp);
    let other = Digest::of(b"other").to_string();
    env.warm(env.list("2026-10-30T00:00:00Z", &[OLD_KEY_ID], &[&other]));
    let gate = gate_from_cache(&env, &vp, "2026-11-02T00:00:00Z").unwrap();
    let record = gate.record_trust(&vp, &spec, FETCHED).unwrap();
    gate.check(&spec).unwrap();
    let list = env.at(offline_source(), FETCHED).cached().unwrap().list;
    assert_eq!(list.flag(&record), None);
}

#[test]
fn tm27_a_stale_cache_still_blocks_what_it_revokes() {
    let env = Env::new();
    let trees = tempfile::tempdir().unwrap();
    let manifest = Digest::of(b"plugin manifest").to_string();
    let vp = install(trees.path(), &manifest, NEW_KEY_ID);
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&manifest]));
    let gate = gate_from_cache(&env, &vp, "2027-06-01T00:00:00Z").unwrap();
    assert!(gate.check(&spec_for(&vp)).is_err());
}

#[test]
fn a_tampered_cache_leaves_no_gate_to_spawn_with() {
    let env = Env::new();
    let trees = tempfile::tempdir().unwrap();
    let manifest = Digest::of(b"plugin manifest").to_string();
    let vp = install(trees.path(), &manifest, NEW_KEY_ID);
    env.warm(env.list("2026-10-30T00:00:00Z", &[], &[&manifest]));
    let file = env.at(offline_source(), FETCHED).cache_file();
    edit_cache(&file, &format!("\\\"{manifest}\\\""), "");
    assert!(matches!(
        gate_from_cache(&env, &vp, FETCHED),
        Err(RevocationError::CacheInvalid { .. })
    ));
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
    let ids: Vec<String> = store.pinned_map().unwrap()["acme"]
        .iter()
        .map(PublicKey::key_id)
        .collect();
    assert_eq!(ids, vec![NEW_KEY_ID.to_owned()]);

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
    let ids: Vec<String> = store.pinned_map().unwrap()["acme"]
        .iter()
        .map(PublicKey::key_id)
        .collect();
    assert_eq!(ids, vec![OLD_KEY_ID.to_owned()]);
}

fn rotation_json(old: &PublicKey, new: &PublicKey, not_before: &str) -> Vec<u8> {
    format!(
        "{{\"namespace\":\"acme\",\"newKeyId\":\"{}\",\"notBefore\":\"{not_before}\",\"oldKeyId\":\"{}\",\"schema_version\":1,\"type\":\"rotation\"}}",
        new.key_id(),
        old.key_id()
    )
    .into_bytes()
}

fn rotate(store: &TrustStore, old: &Root, new: &Root, not_before: &str) -> Result<(), TrustError> {
    let statement = rotation_json(&old.public(), &new.public(), not_before);
    let (old_sig, new_sig) = (old.sign(&statement), new.sign(&statement));
    let new_key = new.public();
    let p = RotationProof {
        statement: &statement,
        old_signature: &old_sig,
        new_signature: &new_sig,
        new_key: &new_key,
    };
    store
        .rotate_key(
            "acme",
            &p,
            &RevocationList::default(),
            "2026-12-31T00:00:00Z",
        )
        .map(drop)
}

#[test]
fn a_second_rotation_must_be_newer_than_the_first() {
    let (a, b, c) = (Root::new(), Root::new(), Root::new());
    let state = tempfile::tempdir().unwrap();
    let store = TrustStore::open(state.path());
    store
        .pin_key("acme", &a.public().to_base64(), FETCHED)
        .unwrap();
    rotate(&store, &a, &b, "2026-11-10T00:00:00Z").unwrap();

    for older in ["2026-11-10T00:00:00Z", "2026-11-01T00:00:00Z"] {
        let err = rotate(&store, &b, &c, older).unwrap_err();
        assert!(
            matches!(err, TrustError::Rotation(SignError::Rollback(_))),
            "{older}: {err:?}"
        );
    }
    rotate(&store, &b, &c, "2026-11-10T00:00:01Z").unwrap();
    let ids: Vec<String> = store.pinned_map().unwrap()["acme"]
        .iter()
        .map(PublicKey::key_id)
        .collect();
    assert_eq!(ids, vec![c.public().key_id()]);
}
