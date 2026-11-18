//! The boundary between the two pack id types: the model's `PackId` and the lock file's.
//!
//! Both follow `common.schema.json#/$defs/PackId`. The lock type also caps the length, so a
//! model id longer than the cap does not convert. The two types merge in a later change.

use slopvac_model::PackId;
use slopvac_plugins::lock::PackId as LockPackId;

/// A pack id that the other type refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a pack id")]
pub struct PackIdError;

/// Convert the lock file's pack id to the model's.
pub fn from_lock_id(id: &LockPackId) -> Result<PackId, PackIdError> {
    PackId::new(id.as_str()).map_err(|_| PackIdError)
}

/// Convert the model's pack id to the lock file's.
pub fn to_lock_id(id: &PackId) -> Result<LockPackId, PackIdError> {
    id.as_str().parse().map_err(|_| PackIdError)
}
