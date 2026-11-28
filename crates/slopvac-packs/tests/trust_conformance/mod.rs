//! The cases every [`TrustQuery`] answers the same way, for any store behind it.
//!
//! Each case builds real installed trees in temporary directories, with canonical roots and
//! hashes measured from the files, and runs through a [`TrustStore`] that records trust for
//! the installations the case names. The test double runs them now, and the store adapter runs
//! the same cases when it lands.

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_packs::{MANIFEST_FILE, Manifest, Trust, TrustQuery, TrustQueryError, TrustSubject};
use slopvac_plugins::extract::{TreeHash, tree_hash};
use slopvac_plugins::lock::{LockedPack, Lockfile};

/// A trust store that a case can fill.
pub(crate) trait TrustStore {
    /// A store that records trust for each installation in `trusted`, and for nothing else.
    fn trusting(&self, trusted: &[TrustSubject<'_>]) -> Box<dyn TrustQuery>;
    /// A store that cannot be read.
    fn failing(&self) -> Box<dyn TrustQuery>;
}

const MANIFEST: &str = "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[provides]\n\n[plugin]\ncommand = \"bin/acme\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }\n";

/// One code plugin, `acme`, installed with the same files at two roots.
struct Installs {
    _dirs: [tempfile::TempDir; 2],
    locked: LockedPack,
    manifest: Manifest,
    roots: [PathBuf; 2],
    hashes: [TreeHash; 2],
}

impl Installs {
    fn new() -> Self {
        let lock = Lockfile::parse(&format!(
            "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\nsource = \"dist/acme.tar.gz\"\ndigest = \"sha256:{}\"\nrequires = []\n",
            "1".repeat(64)
        ))
        .unwrap();
        let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        let roots = dirs.each_ref().map(|dir| {
            let root = fs::canonicalize(dir.path()).unwrap().join("tree");
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::write(root.join(MANIFEST_FILE), MANIFEST).unwrap();
            fs::write(root.join("bin/acme"), "#!/bin/sh\n").unwrap();
            root
        });
        let hashes = roots.each_ref().map(|root| tree_hash(root).unwrap());
        assert_eq!(hashes[0], hashes[1], "the same content at both roots");
        Self {
            _dirs: dirs,
            locked: lock.packs()[0].clone(),
            manifest: Manifest::parse(MANIFEST).unwrap(),
            roots,
            hashes,
        }
    }

    /// The installation at root `root`, with the hash measured from its files.
    fn at(&self, root: usize) -> TrustSubject<'_> {
        TrustSubject::new(
            &self.locked,
            Path::new(&self.roots[root]),
            &self.hashes[root],
            MANIFEST.as_bytes(),
            &self.manifest,
        )
    }

    /// Change a file of the tree at root `root`, and measure its hash again.
    fn change(&mut self, root: usize) {
        fs::write(self.roots[root].join("bin/acme"), "#!/bin/sh\nexit 1\n").unwrap();
        self.hashes[root] = tree_hash(&self.roots[root]).unwrap();
    }
}

/// An installation the store trusts is trusted.
pub(crate) fn trusted(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[installs.at(0)]);
    assert_eq!(query.trust(&installs.at(0)), Ok(Trust::Trusted));
}

/// An installation of a plugin the store has no record for is untrusted.
pub(crate) fn untrusted(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[]);
    assert_eq!(query.trust(&installs.at(0)), Ok(Trust::Untrusted));
}

/// A trusted installation whose files changed after trust, at the same root, changed.
pub(crate) fn changed(store: &dyn TrustStore) {
    let mut installs = Installs::new();
    let query = store.trusting(&[installs.at(0)]);
    installs.change(0);
    assert_eq!(query.trust(&installs.at(0)), Ok(Trust::Changed));
}

/// The trusted content at another root is untrusted: trust covers one installation.
pub(crate) fn another_root(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[installs.at(0)]);
    assert_eq!(query.trust(&installs.at(1)), Ok(Trust::Untrusted));
}

/// A store that cannot be read answers with an error, never with a missing approval.
pub(crate) fn store_error(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.failing();
    assert_eq!(
        query.trust(&installs.at(0)),
        Err(TrustQueryError::StoreUnreadable)
    );
}
