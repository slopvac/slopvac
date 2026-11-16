//! `plugins update`: resolve within config ranges and write a new lock (bead
//! 8l4.9.30; threat model K1-K3 rollback floor and key pins, K5 revocation;
//! policies.md offline mode and exit code 2).

mod oci_server;
mod revocations;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::GzEncoder;
use oci_server::{Response, Server};
use revocations::{ListServer, Root, Signed, list_json};
use slopvac_plugins::lock::{Lockfile, PackId, SourceKind, Version, VersionReq, parse_version_req};
use slopvac_plugins::oci::{Anonymous, Client, Descriptor, Digest, MEDIA_MANIFEST, Manifest};
use slopvac_plugins::revoke::Revocations;
use slopvac_plugins::sign::{
    ANNOTATION_ID, ANNOTATION_VERSION, KeyPolicy, MEDIA_MINISIG, MEDIA_STATEMENT, PublicKey,
    SignError, Statement,
};
use slopvac_plugins::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER, Network, SyncError};
use slopvac_plugins::update::{Outcome, UpdateError, UpdateReport, Updater};

const FLOOR: &str = "2026-11-01T00:00:00Z";
const LATER: &str = "2026-11-10T00:00:00Z";
const EARLIER: &str = "2026-10-01T00:00:00Z";
const NOW: &str = "2026-11-14T00:00:00Z";

struct Key {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl Key {
    fn new() -> Self {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            pk: kp.pk,
            sk: kp.sk,
        }
    }
    fn public(&self) -> PublicKey {
        PublicKey::from_base64(&self.pk.to_base64()).unwrap()
    }
    fn sign(&self, bytes: &[u8]) -> String {
        minisign::sign(Some(&self.pk), &self.sk, Cursor::new(bytes), None, None)
            .unwrap()
            .to_string()
    }
}

fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, name, *data).unwrap();
    }
    let raw = builder.into_inner().unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

/// One signed version of a pack, ready to serve.
struct Release {
    tag: String,
    manifest: Vec<u8>,
    digest: String,
    blobs: HashMap<String, Vec<u8>>,
    key_id: String,
    not_before: String,
}

fn release(key: &Key, id: &str, version: &str, not_before: &str) -> Release {
    let layer = targz(&[("pack.toml", format!("version = \"{version}\"\n").as_bytes())]);
    let mut manifest = Manifest {
        schema_version: 2,
        media_type: Some(MEDIA_MANIFEST.into()),
        artifact_type: Some(ARTIFACT_TYPE_PACK.into()),
        config: Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{}"),
        layers: vec![Descriptor::for_bytes(MEDIA_PACK_LAYER, &layer)],
        subject: None,
        annotations: BTreeMap::from([
            (ANNOTATION_ID.to_owned(), id.to_owned()),
            (ANNOTATION_VERSION.to_owned(), version.to_owned()),
        ]),
    };
    let key_id = key.public().key_id();
    let statement = Statement::for_manifest("acme", id, version, &key_id, not_before, &manifest)
        .canonical_bytes();
    let minisig = key.sign(&statement);
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
    let body = serde_json::to_vec(&manifest).unwrap();
    let mut blobs = HashMap::new();
    blobs.insert(Digest::of(&layer).to_string(), layer);
    blobs.insert(Digest::of(&statement).to_string(), statement);
    blobs.insert(
        Digest::of(minisig.as_bytes()).to_string(),
        minisig.into_bytes(),
    );
    blobs.insert(Digest::of(b"{}").to_string(), b"{}".to_vec());
    Release {
        tag: version.to_owned(),
        digest: Digest::of(&body).to_string(),
        manifest: body,
        blobs,
        key_id,
        not_before: not_before.to_owned(),
    }
}

/// A served repository: path prefix, (tag, digest, manifest) and blobs.
type Repo = (
    String,
    Vec<(String, String, Vec<u8>)>,
    HashMap<String, Vec<u8>>,
);

/// A registry with the repositories `acme/<name>`, each with its releases.
fn registry(repos: &[(&str, &[&Release])]) -> Server {
    let repos: Vec<Repo> = repos
        .iter()
        .map(|(name, releases)| {
            let manifests = releases
                .iter()
                .map(|r| (r.tag.clone(), r.digest.clone(), r.manifest.clone()))
                .collect();
            let blobs = releases.iter().flat_map(|r| r.blobs.clone()).collect();
            (format!("/v2/acme/{name}/"), manifests, blobs)
        })
        .collect();
    Server::start(move |req| {
        let Some((prefix, manifests, blobs)) = repos
            .iter()
            .find(|(p, _, _)| req.path.starts_with(p.as_str()))
        else {
            return Response::new(404);
        };
        let rest = &req.path[prefix.len()..];
        if rest.starts_with("tags/list") {
            let tags: Vec<&str> = manifests.iter().map(|(t, _, _)| t.as_str()).collect();
            return Response::new(200).json(&serde_json::json!({ "tags": tags }));
        }
        if let Some(reference) = rest.strip_prefix("manifests/") {
            return manifests
                .iter()
                .find(|(t, d, _)| t == reference || d == reference)
                .map_or_else(
                    || Response::new(404),
                    |(_, _, body)| {
                        Response::new(200)
                            .header("Content-Type", MEDIA_MANIFEST)
                            .body(body.clone())
                    },
                );
        }
        let digest = rest.rsplit('/').next().unwrap_or_default();
        blobs.get(digest).map_or_else(
            || Response::new(404),
            |b| Response::new(200).body(b.clone()),
        )
    })
}

fn client() -> Client {
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
}

fn policy(keys: &[&Key]) -> KeyPolicy {
    KeyPolicy {
        pinned: BTreeMap::from([("acme".to_owned(), keys.iter().map(|k| k.public()).collect())]),
        ..KeyPolicy::default()
    }
}

fn pack_id(s: &str) -> PackId {
    s.parse().unwrap()
}

fn ranges(entries: &[(&str, &str)]) -> BTreeMap<PackId, VersionReq> {
    entries
        .iter()
        .map(|(id, req)| (pack_id(id), parse_version_req(req).unwrap()))
        .collect()
}

/// A lock entry for `current`, signed in the lock when `signed`.
fn entry(id: &str, registry: &str, current: &Release, signed: bool) -> String {
    let mut text = format!(
        "\n[[packs]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"{}\"\nslopvac_api = 1\n\
         source = \"oci://{registry}/acme/{id}:{}\"\ndigest = \"{}\"\n",
        current.tag, current.tag, current.digest
    );
    if signed {
        write!(
            text,
            "\n[packs.signature]\nkey_id = \"{}\"\nnot_before = \"{}\"\n",
            current.key_id, current.not_before
        )
        .unwrap();
    }
    text
}

struct Env {
    dir: tempfile::TempDir,
    root: Root,
    list: ListServer,
}

impl Env {
    /// The revocation source serves an empty list signed by the root.
    fn new() -> Self {
        let root = Root::new();
        let empty = Signed::by(
            &root,
            list_json("2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z", &[], &[]),
        );
        Self {
            dir: tempfile::tempdir().unwrap(),
            root,
            list: ListServer::start(Some(empty)),
        }
    }
    fn lock_path(&self) -> PathBuf {
        self.dir.path().join("packs.lock")
    }
    fn write_lock(&self, entries: &[String]) -> String {
        let mut text = String::from("schema_version = 3\n");
        for e in entries {
            text.push_str(e);
        }
        let canonical = Lockfile::parse(&text).unwrap().to_toml();
        fs::write(self.lock_path(), &canonical).unwrap();
        canonical
    }
    fn lock(&self) -> Lockfile {
        Lockfile::parse(&fs::read_to_string(self.lock_path()).unwrap()).unwrap()
    }
    fn revoke(&self, keys: &[&str], digests: &[&str]) {
        self.list.serve(Some(Signed::by(
            &self.root,
            list_json(
                "2026-11-10T00:00:00Z",
                "2026-12-10T00:00:00Z",
                keys,
                digests,
            ),
        )));
    }
    fn update(
        &self,
        network: Network<'_>,
        policy: &KeyPolicy,
        ranges: &BTreeMap<PackId, VersionReq>,
        only: &[PackId],
    ) -> Result<UpdateReport, UpdateError> {
        let revocations = Revocations::new(
            self.dir.path().join("state"),
            vec![self.root.public()],
            self.list.source(),
            NOW,
        )
        .unwrap();
        Updater::new(network, policy, &revocations).update(&self.lock_path(), ranges, only)
    }
}

fn version(s: &str) -> Version {
    s.parse().unwrap()
}

#[test]
fn an_in_range_update_writes_the_newest_version_to_the_lock() {
    let key = Key::new();
    let (v120, v130, v140) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
        release(&key, "style", "1.4.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130, &v140])]);
    let reg = server.registry();
    let env = Env::new();
    env.write_lock(&[entry("style", &reg, &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert_eq!(report.exit_code(), 0);
    assert!(report.wrote_lock);
    assert!(
        matches!(&report.packs[0].outcome, Outcome::Updated { to } if *to == version("1.4.0")),
        "{:?}",
        report.packs[0]
    );
    let lock = env.lock();
    let pack = lock.get(&pack_id("style")).unwrap();
    assert_eq!(pack.version, version("1.4.0"));
    assert_eq!(pack.digest.as_str(), v140.digest);
    assert_eq!(
        pack.source.to_string(),
        format!("oci://{reg}/acme/style:1.4.0")
    );
    assert!(matches!(pack.source.kind(), SourceKind::Oci(_)));
    let sig = pack.signature.as_ref().unwrap();
    assert_eq!(sig.key_id.as_str(), v140.key_id);
    assert_eq!(sig.not_before.as_str(), LATER);
    // Canonical text on disk.
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), lock.to_toml());
}

#[test]
fn a_newer_version_outside_the_range_is_reported_and_not_taken() {
    let key = Key::new();
    let (v120, v130, v200) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
        release(&key, "style", "2.0.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130, &v200])]);
    let env = Env::new();
    env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert_eq!(report.packs[0].outside_range, Some(version("2.0.0")));
    assert!(matches!(report.packs[0].outcome, Outcome::Updated { .. }));
    assert_eq!(env.lock().packs()[0].version, version("1.3.0"));
}

#[test]
fn only_a_newer_version_outside_the_range_leaves_the_lock_alone() {
    let key = Key::new();
    let (v120, v200) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "2.0.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v200])]);
    let env = Env::new();
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert!(matches!(report.packs[0].outcome, Outcome::Unchanged));
    assert_eq!(report.packs[0].outside_range, Some(version("2.0.0")));
    assert!(!report.wrote_lock);
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn a_prerelease_is_not_taken_unless_the_range_names_it() {
    let key = Key::new();
    let (v130, rc) = (
        release(&key, "style", "1.3.0", FLOOR),
        release(&key, "style", "1.4.0-rc.1", LATER),
    );
    let server = registry(&[("style", &[&v130, &rc])]);
    let env = Env::new();
    env.write_lock(&[entry("style", &server.registry(), &v130, true)]);
    let c = client();

    let plain = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.3")]),
            &[],
        )
        .unwrap();
    assert!(matches!(plain.packs[0].outcome, Outcome::Unchanged));
    assert_eq!(
        plain.packs[0].outside_range, None,
        "never advertises a prerelease"
    );
    assert_eq!(env.lock().packs()[0].version, version("1.3.0"));

    let named = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", ">=1.3.0, <=1.4.0-rc.1")]),
            &[],
        )
        .unwrap();
    assert!(matches!(named.packs[0].outcome, Outcome::Updated { .. }));
    assert_eq!(env.lock().packs()[0].version, version("1.4.0-rc.1"));
}

#[test]
fn a_key_that_is_not_pinned_is_refused_and_the_lock_stays() {
    let (pinned, rogue) = (Key::new(), Key::new());
    let v120 = release(&pinned, "style", "1.2.0", FLOOR);
    let v130 = release(&rogue, "style", "1.3.0", LATER);
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&pinned]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert!(
        matches!(
            &report.packs[0].outcome,
            Outcome::Refused(SyncError::Sign {
                source: SignError::KeyNotPinned { .. },
                ..
            })
        ),
        "{:?}",
        report.packs[0]
    );
    assert_eq!(report.exit_code(), 2);
    assert!(!report.wrote_lock);
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn a_revoked_artifact_is_refused() {
    let key = Key::new();
    let (v120, v130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    env.revoke(&[], &[&v130.digest]);
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert!(
        matches!(
            &report.packs[0].outcome,
            Outcome::Refused(SyncError::Revoked { .. })
        ),
        "{:?}",
        report.packs[0]
    );
    assert_eq!(report.exit_code(), 2);
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn a_revoked_signing_key_is_refused() {
    let key = Key::new();
    let (v120, v130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    env.revoke(&[&v130.key_id], &[]);
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert!(
        matches!(
            &report.packs[0].outcome,
            Outcome::Refused(SyncError::Revoked { .. })
        ),
        "{:?}",
        report.packs[0]
    );
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn a_statement_older_than_the_locks_floor_is_refused() {
    let key = Key::new();
    let v120 = release(&key, "style", "1.2.0", FLOOR);
    // A higher version whose signed notBefore predates the lock's floor.
    let stale = release(&key, "style", "1.3.0", EARLIER);
    let server = registry(&[("style", &[&v120, &stale])]);
    let env = Env::new();
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap();

    assert!(
        matches!(
            &report.packs[0].outcome,
            Outcome::Refused(SyncError::Sign {
                source: SignError::Rollback(_),
                ..
            })
        ),
        "{:?}",
        report.packs[0]
    );
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn offline_update_is_refused_with_exit_2_and_touches_nothing() {
    let key = Key::new();
    let v120 = release(&key, "style", "1.2.0", FLOOR);
    let server = registry(&[("style", &[&v120])]);
    let env = Env::new();
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);

    let err = env
        .update(
            Network::Offline,
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[],
        )
        .unwrap_err();

    assert!(matches!(err, UpdateError::Offline), "{err:?}");
    assert_eq!(err.exit_code(), 2);
    assert_eq!(err.code(), "offline-update");
    assert!(server.requests().is_empty());
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}

#[test]
fn a_second_update_changes_nothing() {
    let key = Key::new();
    let (v120, v130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();
    let pol = policy(&[&key]);
    let wanted = ranges(&[("style", "^1.2")]);

    let first = env.update(Network::Online(&c), &pol, &wanted, &[]).unwrap();
    let after_first = fs::read_to_string(env.lock_path()).unwrap();
    let second = env.update(Network::Online(&c), &pol, &wanted, &[]).unwrap();

    assert!(first.wrote_lock);
    assert!(!second.wrote_lock);
    assert!(matches!(second.packs[0].outcome, Outcome::Unchanged));
    assert_eq!(second.exit_code(), 0);
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), after_first);
}

#[test]
fn a_named_pack_updates_alone() {
    let key = Key::new();
    let (s120, s130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let (o100, o110) = (
        release(&key, "other", "1.0.0", FLOOR),
        release(&key, "other", "1.1.0", LATER),
    );
    let server = registry(&[("style", &[&s120, &s130]), ("other", &[&o100, &o110])]);
    let reg = server.registry();
    let env = Env::new();
    env.write_lock(&[
        entry("style", &reg, &s120, true),
        entry("other", &reg, &o100, true),
    ]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2"), ("other", "^1.0")]),
            &[pack_id("style")],
        )
        .unwrap();

    assert_eq!(report.packs.len(), 1);
    let lock = env.lock();
    assert_eq!(
        lock.get(&pack_id("style")).unwrap().version,
        version("1.3.0")
    );
    assert_eq!(
        lock.get(&pack_id("other")).unwrap().version,
        version("1.0.0")
    );
}

#[test]
fn a_refused_pack_does_not_block_the_others() {
    let (key, rogue) = (Key::new(), Key::new());
    let (s120, s130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&rogue, "style", "1.3.0", LATER),
    );
    let (o100, o110) = (
        release(&key, "other", "1.0.0", FLOOR),
        release(&key, "other", "1.1.0", LATER),
    );
    let server = registry(&[("style", &[&s120, &s130]), ("other", &[&o100, &o110])]);
    let reg = server.registry();
    let env = Env::new();
    env.write_lock(&[
        entry("style", &reg, &s120, true),
        entry("other", &reg, &o100, true),
    ]);
    let c = client();

    let report = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2"), ("other", "^1.0")]),
            &[],
        )
        .unwrap();

    assert_eq!(report.exit_code(), 2);
    let lock = env.lock();
    assert_eq!(
        lock.get(&pack_id("style")).unwrap().version,
        version("1.2.0")
    );
    assert_eq!(
        lock.get(&pack_id("other")).unwrap().version,
        version("1.1.0")
    );
}

#[test]
fn a_pack_not_in_the_lock_is_an_error() {
    let key = Key::new();
    let v120 = release(&key, "style", "1.2.0", FLOOR);
    let server = registry(&[("style", &[&v120])]);
    let env = Env::new();
    env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let err = env
        .update(
            Network::Online(&c),
            &policy(&[&key]),
            &ranges(&[("style", "^1.2")]),
            &[pack_id("ghost")],
        )
        .unwrap_err();

    assert!(matches!(err, UpdateError::UnknownPack(ref id) if id.as_str() == "ghost"));
}

#[test]
fn an_unsigned_entry_is_signed_by_update_once_the_key_is_pinned() {
    let key = Key::new();
    let (v120, v130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    env.write_lock(&[entry("style", &server.registry(), &v120, false)]);
    let c = client();

    env.update(
        Network::Online(&c),
        &policy(&[&key]),
        &ranges(&[("style", "^1.2")]),
        &[],
    )
    .unwrap();

    let lock = env.lock();
    let sig = lock.packs()[0].signature.as_ref().unwrap();
    assert_eq!(sig.key_id.as_str(), v130.key_id);
    assert_eq!(sig.not_before.as_str(), LATER);
}

#[test]
fn a_pack_without_a_range_is_left_alone() {
    let key = Key::new();
    let (v120, v130) = (
        release(&key, "style", "1.2.0", FLOOR),
        release(&key, "style", "1.3.0", LATER),
    );
    let server = registry(&[("style", &[&v120, &v130])]);
    let env = Env::new();
    let before = env.write_lock(&[entry("style", &server.registry(), &v120, true)]);
    let c = client();

    let report = env
        .update(Network::Online(&c), &policy(&[&key]), &BTreeMap::new(), &[])
        .unwrap();

    assert!(matches!(report.packs[0].outcome, Outcome::Unchanged));
    assert_eq!(fs::read_to_string(env.lock_path()).unwrap(), before);
}
