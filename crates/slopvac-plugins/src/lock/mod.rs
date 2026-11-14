//! The `.slopvac/packs.lock` file (contract: `docs/contract/packs.md`, section 7).
//!
//! Everything here is pure: no network, no file system, no extraction. The sync
//! code produces [`ResolvedPack`] lists and feeds them to [`diff()`]; the update
//! code feeds available versions to [`outdated()`].

mod diff;
mod model;
mod outdated;
mod parse;
mod write;

pub use diff::{Change, Delta, LockDiff, ResolvedPack, diff};
pub use model::{
    Digest, KeyId, LockError, LockedPack, Lockfile, NotBefore, PackId, PackKind, Signature, Source,
    ValueError,
};
pub use outdated::{OutdatedPack, outdated, parse_version_req};
pub use semver::{Version, VersionReq};
