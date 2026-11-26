//! A signed pack artifact, and a cache filled the way a successful sync leaves it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use slopvac_plugins::oci::{Descriptor, Digest, MEDIA_MANIFEST, Manifest};
use slopvac_plugins::sign::{
    ANNOTATION_ID, ANNOTATION_VERSION, MEDIA_MINISIG, MEDIA_STATEMENT, PublicKey, Statement,
};
use slopvac_plugins::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER};

/// A key pair that signs test artifacts.
pub(crate) struct Key {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl Key {
    pub(crate) fn new() -> Self {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Self {
            pk: kp.pk,
            sk: kp.sk,
        }
    }

    pub(crate) fn public(&self) -> PublicKey {
        PublicKey::from_base64(&self.pk.to_base64()).unwrap()
    }

    fn sign(&self, bytes: &[u8]) -> String {
        minisign::sign(Some(&self.pk), &self.sk, Cursor::new(bytes), None, None)
            .unwrap()
            .to_string()
    }
}

/// A signed artifact: its manifest, the digest of the manifest and every blob it names.
pub(crate) struct Published {
    pub(crate) digest: String,
    pub(crate) key_id: String,
    manifest: Vec<u8>,
    blobs: Vec<(String, Vec<u8>)>,
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
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&builder.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

/// Sign an artifact of plugin `id` at `version` in `namespace` with `key`.
pub(crate) fn publish(
    key: &Key,
    namespace: &str,
    id: &str,
    version: &str,
    not_before: &str,
    files: &[(&str, &[u8])],
) -> Published {
    let layer = targz(files);
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
    let statement = Statement::for_manifest(namespace, id, version, &key_id, not_before, &manifest)
        .canonical_bytes();
    let minisig = key.sign(&statement);
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
    manifest
        .layers
        .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let blobs = vec![
        (Digest::of(&layer).to_string(), layer),
        (Digest::of(&statement).to_string(), statement),
        (
            Digest::of(minisig.as_bytes()).to_string(),
            minisig.into_bytes(),
        ),
    ];
    Published {
        digest: Digest::of(&manifest_bytes).to_string(),
        key_id,
        manifest: manifest_bytes,
        blobs,
    }
}

impl Published {
    /// Write the manifest and every blob into the store at `root`, under their digests.
    pub(crate) fn cache_into(&self, root: &Path) {
        let blobs = root.join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        let hex = |digest: &str| digest.split_once(':').unwrap().1.to_owned();
        fs::write(blobs.join(hex(&self.digest)), &self.manifest).unwrap();
        for (digest, bytes) in &self.blobs {
            fs::write(blobs.join(hex(digest)), bytes).unwrap();
        }
    }
}
