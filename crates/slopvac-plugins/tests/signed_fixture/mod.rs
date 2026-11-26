//! A signed pack artifact, and a cache filled the way a successful sync leaves it.
//!
//! The two signing keys are fixed and made for tests only: they sign nothing outside this
//! repository's tests, and no policy outside a test pins them.

#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(dead_code, reason = "each test binary uses part of it")]

use std::collections::{BTreeMap, HashMap};
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

/// Test-only key 1, unencrypted.
const PRIMARY_SECRET: &str = "untrusted comment: rsign encrypted secret key\nRWQAAEIyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAcSmBZoUzk5QeROUkudieeZc7nGt0evMg2KDkVzViXMS5p26iDyQMrF4dZfQiksjUwa9hMA1yuN35jOIAQKIimPtv5bjRXTEY1wOfrE8MkYgXtjWcs/PtbNg/ZKoiEo+AYIRTPbrwI4k=\n";
const PRIMARY_PUBLIC: &str = "RWRxKYFmhTOTlF4dZfQiksjUwa9hMA1yuN35jOIAQKIimPtv5bjRXTEY";
/// Test-only key 2, unencrypted.
const SECONDARY_SECRET: &str = "untrusted comment: rsign encrypted secret key\nRWQAAEIyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAUSCIB0INYM37tQy2CwO88yGZqZTfbBfvm3z/tjbarwwDbtnXa+uB/QLcqtxsaLPEIESQOk6K95Uu4Hzms+lijW6eYTQpVaLvAaYFDt610Lhn6NULfE7VbD7vvizTDf/Bn7Q+huIiemc=\n";
const SECONDARY_PUBLIC: &str = "RWRRIIgHQg1gzQLcqtxsaLPEIESQOk6K95Uu4Hzms+lijW6eYTQpVaLv";

/// A key pair that signs test artifacts.
pub(crate) struct Key {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl Key {
    fn fixed(secret: &str, public: &str) -> Self {
        Self {
            pk: minisign::PublicKey::from_base64(public).unwrap(),
            sk: minisign::SecretKeyBox::from_string(secret)
                .unwrap()
                .into_unencrypted_secret_key()
                .unwrap(),
        }
    }

    /// The first test-only key.
    pub(crate) fn primary() -> Self {
        Self::fixed(PRIMARY_SECRET, PRIMARY_PUBLIC)
    }

    /// The second test-only key, for tests that need a key that is not the first.
    pub(crate) fn secondary() -> Self {
        Self::fixed(SECONDARY_SECRET, SECONDARY_PUBLIC)
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
    pub(crate) manifest: Vec<u8>,
    pub(crate) blobs: HashMap<String, Vec<u8>>,
}

pub(crate) fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
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
    let mut blobs = HashMap::new();
    blobs.insert(Digest::of(&layer).to_string(), layer);
    blobs.insert(Digest::of(&statement).to_string(), statement);
    blobs.insert(
        Digest::of(minisig.as_bytes()).to_string(),
        minisig.into_bytes(),
    );
    blobs.insert(Digest::of(b"{}").to_string(), b"{}".to_vec());
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
