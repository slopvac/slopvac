//! A revocation-list registry for tests: a root key, hand-written list JSON and
//! a server that serves the list as an OCI artifact.
#![allow(dead_code, unreachable_pub, missing_docs)]

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use super::oci_server::{Response, Server};
use slopvac_plugins::oci::{Descriptor, Digest, MEDIA_MANIFEST, Manifest, Reference};
use slopvac_plugins::revoke::{ARTIFACT_TYPE_REVOCATIONS, MEDIA_REVOCATIONS};
use slopvac_plugins::sign::{MEDIA_MINISIG, PublicKey};

pub const REPOSITORY: &str = "slopvac/revocations";

/// A minisign key pair made at run time.
pub struct Root {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl Root {
    pub fn new() -> Self {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            pk: kp.pk,
            sk: kp.sk,
        }
    }
    pub fn public(&self) -> PublicKey {
        PublicKey::from_base64(&self.pk.to_base64()).unwrap()
    }
    /// The raw public key: algorithm, key id, Ed25519 key.
    pub fn raw(&self) -> Vec<u8> {
        self.pk.to_bytes()
    }
    pub fn sign(&self, bytes: &[u8]) -> String {
        minisign::sign(Some(&self.pk), &self.sk, Cursor::new(bytes), None, None)
            .unwrap()
            .to_string()
    }
}

/// The canonical list text, written by hand: keys sorted, no white space.
/// `keys` and `digests` must already be in the order the test wants.
pub fn list_json(issued: &str, expires: &str, keys: &[&str], digests: &[&str]) -> Vec<u8> {
    let quoted = |v: &[&str]| {
        v.iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"digests\":[{}],\"expires\":\"{expires}\",\"issued\":\"{issued}\",\"keys\":[{}],\"schema_version\":1,\"type\":\"revocations\"}}",
        quoted(digests),
        quoted(keys)
    )
    .into_bytes()
}

/// One list and its signature.
#[derive(Clone)]
pub struct Signed {
    pub statement: Vec<u8>,
    pub minisig: String,
}

impl Signed {
    pub fn by(key: &Root, statement: Vec<u8>) -> Self {
        let minisig = key.sign(&statement);
        Self { statement, minisig }
    }
}

type Edit = fn(&mut Manifest);

/// Serves whatever list is current at `<registry>/slopvac/revocations:latest`.
pub struct ListServer {
    pub server: Server,
    current: Arc<Mutex<Option<(Signed, Edit)>>>,
}

fn unchanged(_: &mut Manifest) {}

impl ListServer {
    pub fn start(initial: Option<Signed>) -> Self {
        let current = Arc::new(Mutex::new(initial.map(|s| (s, unchanged as Edit))));
        let shared = Arc::clone(&current);
        let server = Server::start(move |req| {
            let Some((signed, edit)) = shared.lock().unwrap().clone() else {
                return Response::new(404);
            };
            let (manifest, blobs) = artifact(&signed, edit);
            if req.path.contains("/manifests/") {
                return Response::new(200)
                    .header("Content-Type", MEDIA_MANIFEST)
                    .body(manifest);
            }
            let last = req.path.rsplit('/').next().unwrap_or_default();
            blobs.get(last).map_or_else(
                || Response::new(404),
                |b| Response::new(200).body(b.clone()),
            )
        });
        Self { server, current }
    }

    pub fn serve(&self, signed: Option<Signed>) {
        *self.current.lock().unwrap() = signed.map(|s| (s, unchanged as Edit));
    }

    /// Serve `signed` in a manifest that `edit` changed after it was built.
    pub fn serve_edited(&self, signed: Signed, edit: Edit) {
        *self.current.lock().unwrap() = Some((signed, edit));
    }

    pub fn source(&self) -> Reference {
        Reference::parse(&format!("{}/{REPOSITORY}:latest", self.server.registry())).unwrap()
    }

    pub fn fetches(&self) -> usize {
        self.server.count(&format!("/v2/{REPOSITORY}/manifests/"))
    }
}

/// The OCI manifest and blobs of a list artifact.
pub fn artifact(signed: &Signed, edit: Edit) -> (Vec<u8>, BTreeMap<String, Vec<u8>>) {
    let mut manifest = Manifest {
        schema_version: 2,
        media_type: Some(MEDIA_MANIFEST.into()),
        artifact_type: Some(ARTIFACT_TYPE_REVOCATIONS.into()),
        config: Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{}"),
        layers: vec![
            Descriptor::for_bytes(MEDIA_REVOCATIONS, &signed.statement),
            Descriptor::for_bytes(MEDIA_MINISIG, signed.minisig.as_bytes()),
        ],
        subject: None,
        annotations: BTreeMap::new(),
    };
    edit(&mut manifest);
    let mut blobs = BTreeMap::new();
    blobs.insert(
        Digest::of(&signed.statement).to_string(),
        signed.statement.clone(),
    );
    blobs.insert(
        Digest::of(signed.minisig.as_bytes()).to_string(),
        signed.minisig.clone().into_bytes(),
    );
    blobs.insert(Digest::of(b"{}").to_string(), b"{}".to_vec());
    (serde_json::to_vec(&manifest).unwrap(), blobs)
}
