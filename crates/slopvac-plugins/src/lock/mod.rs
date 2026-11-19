//! The `.slopvac/plugins.lock` file (contract: `docs/contract/plugins.md`, section 7).
//!
//! # Limits
//!
//! The reader is a trust boundary: a pull request can change the file. It
//! rejects input over [`MAX_LOCK_BYTES`] (1 MiB) before it parses TOML, then
//! more than [`MAX_PACKS`] (1,000) packs, more than [`MAX_REQUIRES`] (256)
//! `requires` entries in a pack and any string over [`MAX_STRING_BYTES`]
//! (4 KiB; pack ids, sources and versions are checked wherever they are built).
//! `slopvac_api` runs from 1 to [`MAX_API_VERSION`] (`i64::MAX`, the largest
//! TOML integer). [`Lockfile::new`] also refuses entries whose canonical text
//! would pass [`MAX_LOCK_BYTES`], because escaping can make text longer. Values
//! under ignored `x-` keys are bounded only by the input cap. Text from the file that appears in an error is escaped, so control
//! and bidirectional characters cannot forge output.
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
    Digest, KeyId, LockError, LockedPack, Lockfile, MAX_API_VERSION, MAX_LOCK_BYTES, MAX_PACKS,
    MAX_REQUIRES, MAX_STRING_BYTES, NotBefore, PackId, PackKind, Signature, Source, SourceKind,
    ValueError,
};
pub use outdated::{OutdatedPack, outdated, parse_version_req};
pub use parse::LOCK_SCHEMA_VERSION;
pub use semver::{Version, VersionReq};
