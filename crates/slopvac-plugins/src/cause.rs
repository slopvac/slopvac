//! Failures of library operations as the text and code that `slopvac plugins` reports.
//!
//! Each function keeps the sentence of the variants whose fields are typed and path free, shows
//! a typed path under the project root (`<external>/<file name>` outside it), and renders any
//! other variant as the withheld text: its free text can name a local path, and the code still
//! tells the user which failure it was.

use std::path::Path;

use slopvac_fs::display_under_root;
use slopvac_messages::render;

use crate::messages::cli::{CliText, Reported};
use crate::messages::sync::{cache_tampered, update_busy};
use crate::messages::trust::{revoke_cache_damaged, store_damaged};
use crate::revoke::RevocationError;
use crate::sync::SyncError;
use crate::trust::TrustError;
use crate::update::UpdateError;

fn withheld() -> String {
    render(&CliText::Withheld)
}

fn under(path: &Path, root: &Path) -> String {
    display_under_root(path, root)
}

/// A revocation failure.
pub(crate) fn revocation(root: &Path, e: &RevocationError) -> Reported {
    let text = match e {
        RevocationError::CacheInvalid { path, why } => {
            revoke_cache_damaged(Path::new(&under(path, root)), why)
        }
        RevocationError::NoRootKey
        | RevocationError::TooLarge
        | RevocationError::BadSignature
        | RevocationError::Expired { .. }
        | RevocationError::Rollback { .. }
        | RevocationError::Equivocation { .. }
        | RevocationError::Clock(_) => e.to_string(),
        _ => withheld(),
    };
    Reported::new(e.code(), text)
}

/// A sync or status failure.
pub(crate) fn sync(root: &Path, e: &SyncError) -> Reported {
    let text = match e {
        SyncError::Revocation(inner) => return revocation(root, inner),
        SyncError::CacheTampered { id, path } => cache_tampered(id, Path::new(&under(path, root))),
        SyncError::OfflinePackMissing { .. }
        | SyncError::Unsigned { .. }
        | SyncError::DigestMismatch { .. }
        | SyncError::Revoked { .. }
        | SyncError::LockMismatch(_) => e.to_string(),
        _ => withheld(),
    };
    Reported::new(e.code(), text)
}

/// An update failure.
pub(crate) fn update(root: &Path, e: &UpdateError) -> Reported {
    let text = match e {
        UpdateError::Revocation(inner) => return revocation(root, inner),
        UpdateError::Busy { path, timeout } => {
            update_busy(&under(Path::new(path), root), timeout.as_secs_f32())
        }
        UpdateError::Offline | UpdateError::UnknownPlugin(_) => e.to_string(),
        _ => withheld(),
    };
    Reported::new(e.code(), text)
}

/// A trust store failure: the reported failure when it has a code, else the sentence alone.
pub(crate) fn store(root: &Path, e: &TrustError) -> Result<Reported, String> {
    let text = match e {
        TrustError::Revocation(inner) => return Ok(revocation(root, inner)),
        TrustError::Corrupt { path, why } => store_damaged(Path::new(&under(path, root)), why),
        TrustError::NotAHash(_)
        | TrustError::NamespaceBound { .. }
        | TrustError::ReservedNamespace { .. }
        | TrustError::NotPinned(_)
        | TrustError::Revoked(_)
        | TrustError::TooLarge { .. } => e.to_string(),
        _ => withheld(),
    };
    match e.code() {
        Some(code) => Ok(Reported::new(code, text)),
        None => Err(text),
    }
}
