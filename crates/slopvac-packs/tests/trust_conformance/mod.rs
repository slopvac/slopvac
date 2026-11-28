//! The cases every [`TrustQuery`] answers the same way, for any store behind it.
//!
//! A store implementation runs each case through a [`TrustStore`] that records trust for the
//! installations a case names. The test double runs them now, and the store adapter runs the
//! same cases when it lands.

use std::path::{Path, PathBuf};

use slopvac_packs::{Manifest, Trust, TrustQuery, TrustQueryError, TrustSubject};
use slopvac_plugins::extract::TreeHash;
use slopvac_plugins::lock::{LockedPack, Lockfile};

/// A trust store that a case can fill.
pub(crate) trait TrustStore {
    /// A store that records trust for each installation in `trusted`, and for nothing else.
    fn trusting(&self, trusted: &[TrustSubject<'_>]) -> Box<dyn TrustQuery>;
    /// A store that cannot be read.
    fn failing(&self) -> Box<dyn TrustQuery>;
}

/// One code plugin, `acme`, at two roots and with two tree hashes.
struct Installs {
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
        let manifest = Manifest::parse(
            "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[provides]\n\n[plugin]\ncommand = \"bin/acme\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }\n",
        )
        .unwrap();
        let hash = |digit: &str| TreeHash::parse(&format!("sha256:{}", digit.repeat(64))).unwrap();
        Self {
            locked: lock.packs()[0].clone(),
            manifest,
            roots: [
                PathBuf::from("/store/trees/first"),
                PathBuf::from("/store/trees/second"),
            ],
            hashes: [hash("a"), hash("b")],
        }
    }

    /// The installation at root `root` with tree hash `hash`.
    fn at(&self, root: usize, hash: usize) -> TrustSubject<'_> {
        TrustSubject::new(
            &self.locked,
            Path::new(&self.roots[root]),
            &self.hashes[hash],
            &self.manifest,
        )
    }
}

/// An installation the store trusts is trusted.
pub(crate) fn trusted(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[installs.at(0, 0)]);
    assert_eq!(query.trust(&installs.at(0, 0)), Ok(Trust::Trusted));
}

/// An installation of a plugin the store has no record for is untrusted.
pub(crate) fn untrusted(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[]);
    assert_eq!(query.trust(&installs.at(0, 0)), Ok(Trust::Untrusted));
}

/// The trusted root with another tree hash changed after trust.
pub(crate) fn changed(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[installs.at(0, 0)]);
    assert_eq!(query.trust(&installs.at(0, 1)), Ok(Trust::Changed));
}

/// The trusted content at another root changed after trust, as the spawn gate reports a tree
/// that moved.
pub(crate) fn another_root(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.trusting(&[installs.at(0, 0)]);
    assert_eq!(query.trust(&installs.at(1, 0)), Ok(Trust::Changed));
}

/// A store that cannot be read answers with an error, never with a missing approval.
pub(crate) fn store_error(store: &dyn TrustStore) {
    let installs = Installs::new();
    let query = store.failing();
    assert_eq!(
        query.trust(&installs.at(0, 0)),
        Err(TrustQueryError::StoreUnreadable)
    );
}
