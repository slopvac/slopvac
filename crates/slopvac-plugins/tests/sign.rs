//! Signature verification tests (TM-22 to TM-25, TM-30 parts, threat model section 7.3).

mod oci_server;

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use oci_server::{Response, Server};
use serde_json::json;
use slopvac_plugins::oci::{
    Anonymous, Client, Descriptor, MEDIA_MANIFEST, Manifest, OciError, Reference,
};
use slopvac_plugins::sign::{
    ANNOTATION_ID, ANNOTATION_VERSION, Expectation, KeyPolicy, MEDIA_MINISIG, MEDIA_STATEMENT,
    PublicKey, SignError, Statement, canonical_json, fetch_and_verify, release_key,
    signature_layers, verify,
};

const NS: &str = "acme";
const ID: &str = "style";
const ART: &str = "application/vnd.slopvac.pack.v1";
type Edit = Box<dyn Fn(&mut Manifest)>;

const LAYER: &str = "application/vnd.slopvac.pack.layer.v1.tar+gzip";

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
        minisign::sign(
            Some(&self.pk),
            &self.sk,
            Cursor::new(bytes),
            Some("trusted"),
            Some("untrusted"),
        )
        .unwrap()
        .to_string()
    }
}

struct Signed {
    manifest: Manifest,
    statement: Vec<u8>,
    minisig: String,
}

fn base_manifest(layers: &[(&str, &[u8])]) -> Manifest {
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_ID.to_owned(), ID.to_owned());
    annotations.insert(ANNOTATION_VERSION.to_owned(), "1.2.0".to_owned());
    Manifest {
        schema_version: 2,
        media_type: Some(MEDIA_MANIFEST.into()),
        artifact_type: Some(ART.into()),
        config: Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{}"),
        layers: layers
            .iter()
            .map(|(m, b)| Descriptor::for_bytes(m, b))
            .collect(),
        subject: None,
        annotations,
    }
}

fn sign_manifest(
    key: &Key,
    ns: &str,
    id: &str,
    version: &str,
    not_before: &str,
    payload: &[(&str, &[u8])],
) -> Signed {
    let mut manifest = base_manifest(payload);
    manifest.annotations.insert(ANNOTATION_ID.into(), id.into());
    manifest
        .annotations
        .insert(ANNOTATION_VERSION.into(), version.into());
    let st = Statement::for_manifest(
        ns,
        id,
        version,
        &key.public().key_id(),
        not_before,
        &manifest,
    );
    let statement = st.canonical_bytes();
    let minisig = key.sign(&statement);
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
    Signed {
        manifest,
        statement,
        minisig,
    }
}

fn good(key: &Key) -> Signed {
    sign_manifest(
        key,
        NS,
        ID,
        "1.2.0",
        "2026-11-01T00:00:00Z",
        &[(LAYER, b"rules"), (LAYER, b"terms")],
    )
}

fn policy(pinned: &[&Key]) -> KeyPolicy {
    KeyPolicy {
        pinned: BTreeMap::from([(NS.to_owned(), pinned.iter().map(|k| k.public()).collect())]),
        ..KeyPolicy::default()
    }
}

fn expect() -> Expectation<'static> {
    Expectation {
        id: ID,
        namespace: NS,
        version: "1.2.0",
        artifact_type: ART,
        floor: None,
        allow_downgrade: false,
    }
}

fn run(s: &Signed, policy: &KeyPolicy, e: &Expectation<'_>) -> Result<(), SignError> {
    verify(&s.manifest, &s.statement, &s.minisig, policy, e).map(|_| ())
}

#[test]
fn a_correctly_signed_artifact_verifies_and_names_its_key() {
    let key = Key::new();
    let s = good(&key);
    let v = verify(
        &s.manifest,
        &s.statement,
        &s.minisig,
        &policy(&[&key]),
        &expect(),
    )
    .unwrap();
    assert_eq!(v.statement().id, ID);
    assert_eq!(v.key().key_id(), key.public().key_id());
    assert_eq!(v.key().fingerprint().len(), 64);
}

#[test]
fn tm23_any_change_to_the_manifest_after_signing_fails() {
    let key = Key::new();
    let p = policy(&[&key]);
    let edits: Vec<(&str, Edit)> = vec![
        (
            "add a layer",
            Box::new(|m| m.layers.insert(0, Descriptor::for_bytes(LAYER, b"extra"))),
        ),
        (
            "drop a layer",
            Box::new(|m| {
                m.layers.remove(0);
            }),
        ),
        ("reorder layers", Box::new(|m| m.layers.swap(0, 1))),
        (
            "swap a layer",
            Box::new(|m| m.layers[0] = Descriptor::for_bytes(LAYER, b"evil")),
        ),
        (
            "change a layer media type",
            Box::new(|m| m.layers[0].media_type = "text/plain".into()),
        ),
        (
            "swap the config",
            Box::new(|m| {
                m.config = Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{\"x\":1}");
            }),
        ),
        (
            "change the artifact type",
            Box::new(|m| m.artifact_type = Some("application/evil".into())),
        ),
        (
            "edit the id annotation",
            Box::new(|m| {
                m.annotations.insert(ANNOTATION_ID.into(), "other".into());
            }),
        ),
        (
            "edit the version annotation",
            Box::new(|m| {
                m.annotations
                    .insert(ANNOTATION_VERSION.into(), "9.9.9".into());
            }),
        ),
        ("drop the annotations", Box::new(|m| m.annotations.clear())),
    ];
    for (name, edit) in edits {
        let mut s = good(&key);
        edit(&mut s.manifest);
        let err = run(&s, &p, &expect()).expect_err(name);
        assert!(
            matches!(err, SignError::Mismatch(_) | SignError::MissingSignature),
            "{name}: {err:?}"
        );
    }
}

#[test]
fn tampering_with_the_statement_bytes_breaks_the_signature() {
    let key = Key::new();
    let mut s = good(&key);
    let text = String::from_utf8(s.statement.clone())
        .unwrap()
        .replace("1.2.0", "1.2.1");
    s.statement = text.into_bytes();
    assert_eq!(
        run(&s, &policy(&[&key]), &expect()),
        Err(SignError::BadSignature)
    );
}

#[test]
fn tm22_tm24_a_key_that_is_not_pinned_is_refused() {
    let signer = Key::new();
    let other = Key::new();
    let s = good(&signer);
    let err = run(&s, &policy(&[&other]), &expect()).unwrap_err();
    assert!(matches!(err, SignError::KeyNotPinned { .. }), "{err:?}");
    assert!(err.to_string().contains("slopvac plugins trust --key"));
    let err = run(&s, &KeyPolicy::default(), &expect()).unwrap_err();
    assert!(matches!(err, SignError::KeyNotPinned { .. }), "{err:?}");
}

#[test]
fn an_attacker_cannot_claim_a_pinned_key_id_with_their_own_signature() {
    let pinned = Key::new();
    let attacker = Key::new();
    let mut manifest = base_manifest(&[(LAYER, b"rules")]);
    let st = Statement::for_manifest(
        NS,
        ID,
        "1.2.0",
        &pinned.public().key_id(),
        "2026-11-01T00:00:00Z",
        &manifest,
    );
    let statement = st.canonical_bytes();
    let minisig = attacker.sign(&statement);
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
    let s = Signed {
        manifest,
        statement,
        minisig,
    };
    assert_eq!(
        run(&s, &policy(&[&pinned]), &expect()),
        Err(SignError::BadSignature)
    );
}

#[test]
fn tm24_a_first_party_namespace_verifies_only_against_compiled_keys() {
    let compiled = Key::new();
    let user_pinned = Key::new();
    let policy = KeyPolicy {
        first_party: vec![compiled.public()],
        first_party_namespaces: vec!["slopvac".into()],
        pinned: BTreeMap::from([(
            "slopvac".to_owned(),
            vec![user_pinned.public(), compiled.public()],
        )]),
    };
    let e = Expectation {
        id: "core",
        namespace: "slopvac",
        ..expect()
    };
    let by_user = sign_manifest(
        &user_pinned,
        "slopvac",
        "core",
        "1.2.0",
        "2026-11-01T00:00:00Z",
        &[(LAYER, b"x")],
    );
    assert!(matches!(
        run(&by_user, &policy, &e),
        Err(SignError::KeyNotPinned { .. })
    ));
    let by_compiled = sign_manifest(
        &compiled,
        "slopvac",
        "core",
        "1.2.0",
        "2026-11-01T00:00:00Z",
        &[(LAYER, b"x")],
    );
    assert_eq!(run(&by_compiled, &policy, &e), Ok(()));
}

#[test]
fn the_statement_must_be_canonical_json_within_limits() {
    let key = Key::new();
    let p = policy(&[&key]);
    let s = good(&key);
    let pretty = serde_json::to_vec_pretty(
        &serde_json::from_slice::<serde_json::Value>(&s.statement).unwrap(),
    )
    .unwrap();
    let resigned = Signed {
        minisig: key.sign(&pretty),
        statement: pretty,
        manifest: s.manifest.clone(),
    };
    assert!(matches!(
        run(&resigned, &p, &expect()),
        Err(SignError::NotCanonical(_))
    ));

    let dup = br#"{"a":1,"a":2}"#.to_vec();
    let s2 = Signed {
        minisig: key.sign(&dup),
        statement: dup,
        manifest: s.manifest.clone(),
    };
    assert!(matches!(
        run(&s2, &p, &expect()),
        Err(SignError::InvalidStatement(_))
    ));

    let mut extra: serde_json::Value = serde_json::from_slice(&s.statement).unwrap();
    extra["unexpected"] = json!(1);
    let bytes = canonical_json(&extra);
    let s3 = Signed {
        minisig: key.sign(&bytes),
        statement: bytes,
        manifest: s.manifest.clone(),
    };
    assert!(matches!(
        run(&s3, &p, &expect()),
        Err(SignError::InvalidStatement(_))
    ));

    let huge = vec![b' '; 70 * 1024];
    let s4 = Signed {
        minisig: key.sign(&huge),
        statement: huge,
        manifest: s.manifest,
    };
    assert_eq!(run(&s4, &p, &expect()), Err(SignError::StatementTooLarge));
}

#[test]
fn exactly_one_statement_layer_and_one_signature_layer_are_required() {
    let key = Key::new();
    let mut s = good(&key);
    s.manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, b"second"));
    assert_eq!(
        run(&s, &policy(&[&key]), &expect()),
        Err(SignError::DuplicateSignature)
    );
    let mut none = good(&key);
    none.manifest
        .layers
        .retain(|l| l.media_type != MEDIA_MINISIG && l.media_type != MEDIA_STATEMENT);
    assert_eq!(
        signature_layers(&none.manifest).unwrap_err(),
        SignError::MissingSignature
    );
}

#[test]
fn the_requested_id_version_and_type_must_match_the_signed_ones() {
    let key = Key::new();
    let p = policy(&[&key]);
    let s = good(&key);
    for e in [
        Expectation {
            id: "other",
            ..expect()
        },
        Expectation {
            namespace: "mallory",
            ..expect()
        },
        Expectation {
            version: "1.3.0",
            ..expect()
        },
        Expectation {
            artifact_type: "application/vnd.slopvac.plugin.v1",
            ..expect()
        },
    ] {
        assert!(
            matches!(run(&s, &p, &e), Err(SignError::Mismatch(_))),
            "{e:?}"
        );
    }
}

#[test]
fn tm25_an_older_version_or_older_not_before_is_refused_without_the_downgrade_flag() {
    let key = Key::new();
    let p = policy(&[&key]);
    let s = good(&key);
    let floor = |v, t| Expectation {
        floor: Some((v, t)),
        ..expect()
    };
    assert_eq!(run(&s, &p, &floor("1.2.0", "2026-11-01T00:00:00Z")), Ok(()));
    assert_eq!(run(&s, &p, &floor("1.1.0", "2026-10-01T00:00:00Z")), Ok(()));
    assert!(matches!(
        run(&s, &p, &floor("1.3.0", "2026-10-01T00:00:00Z")),
        Err(SignError::Rollback(_))
    ));
    assert!(matches!(
        run(&s, &p, &floor("1.2.0", "2026-11-02T00:00:00Z")),
        Err(SignError::Rollback(_))
    ));
    let allowed = Expectation {
        floor: Some(("1.3.0", "2026-12-01T00:00:00Z")),
        allow_downgrade: true,
        ..expect()
    };
    assert_eq!(run(&s, &p, &allowed), Ok(()));
}

#[test]
fn canonical_json_sorts_keys_at_every_depth_and_drops_whitespace() {
    let v = json!({"b": [1, {"z": 1, "a": "x y"}], "a": null});
    assert_eq!(
        String::from_utf8(canonical_json(&v)).unwrap(),
        r#"{"a":null,"b":[1,{"a":"x y","z":1}]}"#
    );
}

#[test]
fn public_keys_reject_garbage() {
    for bad in ["", "not base64!", "AAAA", "RWQ="] {
        assert!(PublicKey::from_base64(bad).is_err(), "{bad}");
    }
}

#[test]
fn fetch_and_verify_reads_the_signature_blobs_from_the_registry() {
    let key = Key::new();
    let s = good(&key);
    let blobs: Arc<BTreeMap<String, Vec<u8>>> = Arc::new(
        [
            (s.manifest.layers[2].digest.to_string(), s.statement.clone()),
            (
                s.manifest.layers[3].digest.to_string(),
                s.minisig.clone().into_bytes(),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let server = Server::start(move |req| {
        let digest = req.path.rsplit('/').next().unwrap_or_default();
        match blobs.get(digest) {
            Some(b) => Response::new(200).body(b.clone()),
            None => Response::new(404),
        }
    });
    let r = Reference::parse(&format!("{}/acme/style:1.2.0", server.registry())).unwrap();
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let v = fetch_and_verify(&client, &r, &s.manifest, &policy(&[&key]), &expect()).unwrap();
    assert_eq!(v.statement().version, "1.2.0");
    let wrong = fetch_and_verify(&client, &r, &s.manifest, &policy(&[&Key::new()]), &expect());
    assert!(matches!(wrong, Err(SignError::KeyNotPinned { .. })));
}

fn run_not_before(key: &Key, not_before: &str) -> Result<(), SignError> {
    let s = sign_manifest(key, NS, ID, "1.2.0", not_before, &[(LAYER, b"rules")]);
    run(&s, &policy(&[key]), &expect())
}

#[test]
fn not_before_accepts_real_calendar_times() {
    let key = Key::new();
    for t in [
        "2024-02-29T00:00:00Z",
        "2000-02-29T12:30:45Z",
        "2026-12-31T23:59:59Z",
    ] {
        assert_eq!(run_not_before(&key, t), Ok(()), "{t}");
    }
}

#[test]
fn not_before_rejects_times_that_do_not_exist() {
    let key = Key::new();
    for t in [
        "2026-02-29T00:00:00Z", // not a leap year
        "2100-02-29T00:00:00Z", // century, not a leap year
        "2026-04-31T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-01-00T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
    ] {
        assert!(
            matches!(run_not_before(&key, t), Err(SignError::InvalidStatement(_))),
            "{t}"
        );
    }
}

#[test]
fn a_malformed_lockfile_floor_time_fails_closed() {
    let key = Key::new();
    let s = good(&key);
    for t in [
        "",
        "yesterday",
        "2026-02-29T00:00:00Z",
        "2026-11-01T24:00:00Z",
    ] {
        let e = Expectation {
            floor: Some(("1.0.0", t)),
            ..expect()
        };
        assert!(
            matches!(
                run(&s, &policy(&[&key]), &e),
                Err(SignError::InvalidStatement(_))
            ),
            "{t:?}"
        );
    }
}

#[test]
fn a_version_with_build_metadata_is_refused() {
    let key = Key::new();
    let s = sign_manifest(
        &key,
        NS,
        ID,
        "1.2.0+build.5",
        "2026-11-01T00:00:00Z",
        &[(LAYER, b"rules")],
    );
    let e = Expectation {
        version: "1.2.0+build.5",
        ..expect()
    };
    assert!(matches!(
        run(&s, &policy(&[&key]), &e),
        Err(SignError::InvalidStatement(_))
    ));
}

const FIXED_PUB: &str = include_str!("fixtures/sign/minisign.pub");
const FIXED_STATEMENT: &[u8] = include_bytes!("fixtures/sign/statement.json");
const FIXED_SIG: &str = include_str!("fixtures/sign/statement.minisig");
const FIXED_LEGACY_SIG: &str = include_str!("fixtures/sign/statement.legacy.minisig");

fn fixed_key() -> PublicKey {
    PublicKey::from_base64(FIXED_PUB.lines().nth(1).unwrap()).unwrap()
}

fn fixed_manifest() -> Manifest {
    let mut m = base_manifest(&[(LAYER, b"rules")]);
    m.layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, FIXED_STATEMENT));
    m.layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, FIXED_SIG.as_bytes()));
    m
}

#[test]
fn a_fixed_vector_from_the_minisign_cli_has_the_expected_identity_and_verifies() {
    let key = fixed_key();
    assert_eq!(key.key_id(), "684BF07F758E0DD7");
    assert_eq!(
        key.fingerprint(),
        "331d813cac2254b59314d744d5a1f12aad3f7c39598d80f8466fc64655d41e11"
    );
    let st: Statement = serde_json::from_slice(FIXED_STATEMENT).unwrap();
    assert_eq!(st.canonical_bytes(), FIXED_STATEMENT);
    let policy = KeyPolicy {
        pinned: BTreeMap::from([(NS.to_owned(), vec![key])]),
        ..KeyPolicy::default()
    };
    assert_eq!(
        verify(
            &fixed_manifest(),
            FIXED_STATEMENT,
            FIXED_SIG,
            &policy,
            &expect()
        )
        .map(|_| ()),
        Ok(())
    );
}

#[test]
fn a_legacy_non_prehashed_signature_is_refused() {
    let policy = KeyPolicy {
        pinned: BTreeMap::from([(NS.to_owned(), vec![fixed_key()])]),
        ..KeyPolicy::default()
    };
    assert_eq!(
        verify(
            &fixed_manifest(),
            FIXED_STATEMENT,
            FIXED_LEGACY_SIG,
            &policy,
            &expect()
        )
        .map(|_| ()),
        Err(SignError::BadSignature)
    );
}

#[test]
fn a_key_pinned_for_one_namespace_cannot_sign_another() {
    let a = Key::new();
    let b = Key::new();
    let policy = KeyPolicy {
        pinned: BTreeMap::from([
            ("acme".to_owned(), vec![a.public()]),
            ("other".to_owned(), vec![b.public()]),
        ]),
        ..KeyPolicy::default()
    };
    let for_other = |key: &Key| {
        let s = sign_manifest(
            key,
            "other",
            ID,
            "1.2.0",
            "2026-11-01T00:00:00Z",
            &[(LAYER, b"rules")],
        );
        let e = Expectation {
            namespace: "other",
            ..expect()
        };
        run(&s, &policy, &e)
    };
    assert!(matches!(
        for_other(&a),
        Err(SignError::KeyNotPinned { namespace, .. }) if namespace == "other"
    ));
    assert_eq!(for_other(&b), Ok(()));
}

/// A correctly signed statement whose fields `edit` changed after the manifest was bound.
///
/// The statement binds the unsigned manifest; the signature layers are appended afterwards,
/// exactly as in [`sign_manifest`].
fn signed_statement(key: &Key, edit: impl FnOnce(&mut Statement)) -> Signed {
    let mut manifest = base_manifest(&[(LAYER, b"rules"), (LAYER, b"terms")]);
    let mut st = Statement::for_manifest(
        NS,
        ID,
        "1.2.0",
        &key.public().key_id(),
        "2026-11-01T00:00:00Z",
        &manifest,
    );
    edit(&mut st);
    let statement = st.canonical_bytes();
    let minisig = key.sign(&statement);
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
    Signed {
        manifest,
        statement,
        minisig,
    }
}

#[test]
fn an_unedited_signed_statement_verifies() {
    let key = Key::new();
    let s = signed_statement(&key, |_| {});
    assert_eq!(run(&s, &policy(&[&key]), &expect()), Ok(()));
}

#[test]
fn a_signed_statement_with_a_malformed_namespace_or_id_is_refused() {
    let key = Key::new();
    for (field, value) in [
        ("namespace", "Acme"),
        ("namespace", ""),
        ("id", "Style"),
        ("id", "1st"),
    ] {
        let s = signed_statement(&key, |st| match field {
            "namespace" => st.namespace = value.into(),
            _ => st.id = value.into(),
        });
        let mut e = expect();
        let mut pol = policy(&[&key]);
        if field == "namespace" {
            e.namespace = value;
            pol.pinned = BTreeMap::from([(value.to_owned(), vec![key.public()])]);
        } else {
            e.id = value;
        }
        let err = run(&s, &pol, &e).expect_err(value);
        assert!(
            matches!(err, SignError::InvalidStatement(_)),
            "{field}={value:?}: {err:?}"
        );
    }
}

#[test]
fn a_correctly_signed_statement_with_an_unsupported_schema_version_is_refused() {
    let key = Key::new();
    let s = signed_statement(&key, |st| st.schema_version = 2);
    let err = run(&s, &policy(&[&key]), &expect()).unwrap_err();
    assert!(
        matches!(&err, SignError::Mismatch(m) if m.contains("schema_version")),
        "{err:?}"
    );
}

/// Fetch a manifest whose minisig descriptor claims `size` bytes; returns the result and the
/// number of requests the registry saw for that blob.
fn fetch_with_minisig_size(size: usize) -> (Result<(), SignError>, usize) {
    let key = Key::new();
    let mut s = good(&key);
    let padding = vec![b' '; size];
    s.manifest.layers[3] = Descriptor::for_bytes(MEDIA_MINISIG, &padding);
    let minisig_digest = s.manifest.layers[3].digest.to_string();
    let statement = s.statement.clone();
    let statement_digest = s.manifest.layers[2].digest.to_string();
    let server = Server::start(move |req| {
        let digest = req.path.rsplit('/').next().unwrap_or_default();
        if digest == statement_digest {
            Response::new(200).body(statement.clone())
        } else if digest == minisig_digest {
            Response::new(200).body(padding.clone())
        } else {
            Response::new(404)
        }
    });
    let r = Reference::parse(&format!("{}/acme/style:1.2.0", server.registry())).unwrap();
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let result = fetch_and_verify(&client, &r, &s.manifest, &policy(&[&key]), &expect());
    let hits = server.count(&format!(
        "/v2/acme/style/blobs/{}",
        s.manifest.layers[3].digest
    ));
    (result.map(|_| ()), hits)
}

#[test]
fn an_oversized_minisig_descriptor_is_refused_before_any_request() {
    let (result, hits) = fetch_with_minisig_size(4097);
    assert!(
        matches!(
            result,
            Err(SignError::Fetch(OciError::TooLarge { limit: 4096, .. }))
        ),
        "{result:?}"
    );
    assert_eq!(hits, 0);
}

#[test]
fn a_minisig_descriptor_at_the_limit_is_fetched() {
    let (result, hits) = fetch_with_minisig_size(4096);
    assert!(
        !matches!(result, Err(SignError::Fetch(OciError::TooLarge { .. }))),
        "{result:?}"
    );
    assert_eq!(hits, 1);
}

// ---- The compiled first-party key ----

const RELEASE_PUB_FILE: &str = include_str!("../../../keys/slopvac-release.pub");

#[test]
fn the_compiled_release_key_has_the_published_key_id() {
    // The key id is bytes 2..10 of the decoded key line, shown little-endian.
    // Worked out by hand from the base64 line: 6D AC 70 45 DB 4B 91 D9.
    let line = RELEASE_PUB_FILE.lines().nth(1).unwrap();
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, line).unwrap();
    assert_eq!(raw[2..10], [0x6D, 0xAC, 0x70, 0x45, 0xDB, 0x4B, 0x91, 0xD9]);
    assert_eq!(release_key().key_id(), "D9914BDB4570AC6D");
}

fn first_party_signed(key: &Key) -> Signed {
    sign_manifest(
        key,
        "slopvac",
        "core",
        "1.2.0",
        "2026-11-01T00:00:00Z",
        &[(LAYER, b"x")],
    )
}

fn first_party_expect() -> Expectation<'static> {
    Expectation {
        id: "core",
        namespace: "slopvac",
        ..expect()
    }
}

#[test]
fn the_compiled_policy_binds_the_first_party_namespace_to_the_release_key() {
    let policy = KeyPolicy::compiled();
    assert_eq!(policy.first_party.len(), 1);
    assert_eq!(policy.first_party[0].key_id(), "D9914BDB4570AC6D");
    assert_eq!(policy.first_party_namespaces, ["slopvac"]);
    assert!(policy.pinned.is_empty());
}

#[test]
fn a_first_party_pack_signed_by_another_key_is_refused_with_the_compiled_key() {
    let other = Key::new();
    let s = first_party_signed(&other);
    assert!(matches!(
        run(&s, &KeyPolicy::compiled(), &first_party_expect()),
        Err(SignError::KeyNotPinned { .. })
    ));
}

#[test]
fn a_first_party_pack_verifies_only_when_the_signing_key_is_injected() {
    let test_key = Key::new();
    let s = first_party_signed(&test_key);
    let injected = KeyPolicy {
        first_party: vec![test_key.public()],
        ..KeyPolicy::compiled()
    };
    assert_eq!(run(&s, &injected, &first_party_expect()), Ok(()));
}
