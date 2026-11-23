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
use crate::revoke::RevocationError;
use crate::sync::SyncError;
use crate::trust::TrustError;
use crate::update::UpdateError;

/// The sentence that stands in for withheld detail, with the code of the failure it replaces.
/// Every withheld text goes through here, so the code always shows beside it.
pub(crate) fn withheld(code: &str) -> String {
    render(&CliText::Withheld {
        code: code.to_owned(),
    })
}

/// The code of a trust store failure that has none of its own: the group's store failure.
const STORE_FAILED: &str = "plugins-store-failed";

fn under(path: &Path, root: &Path) -> String {
    display_under_root(path, root)
}

/// A revocation failure.
pub(crate) fn revocation(e: &RevocationError) -> Reported {
    let text = match e {
        // The parser's text can quote the file, so only the code is kept.
        RevocationError::CacheInvalid { .. } => e.code().to_owned(),
        RevocationError::NoRootKey
        | RevocationError::TooLarge
        | RevocationError::BadSignature
        | RevocationError::Expired { .. }
        | RevocationError::Rollback { .. }
        | RevocationError::Equivocation { .. }
        | RevocationError::Clock(_) => e.to_string(),
        _ => withheld(e.code()),
    };
    Reported::new(e.code(), text)
}

/// A sync or status failure.
pub(crate) fn sync(root: &Path, e: &SyncError) -> Reported {
    let text = match e {
        SyncError::Revocation(inner) => return revocation(inner),
        SyncError::CacheTampered { id, path } => cache_tampered(id, Path::new(&under(path, root))),
        SyncError::OfflinePackMissing { .. }
        | SyncError::Unsigned { .. }
        | SyncError::DigestMismatch { .. }
        | SyncError::Revoked { .. }
        | SyncError::LockMismatch(_) => e.to_string(),
        _ => withheld(e.code()),
    };
    Reported::new(e.code(), text)
}

/// An update failure.
pub(crate) fn update(root: &Path, e: &UpdateError) -> Reported {
    let text = match e {
        UpdateError::Revocation(inner) => return revocation(inner),
        UpdateError::Busy { path, timeout } => {
            update_busy(&under(Path::new(path), root), timeout.as_secs_f32())
        }
        UpdateError::Offline | UpdateError::UnknownPlugin(_) => e.to_string(),
        _ => withheld(e.code()),
    };
    Reported::new(e.code(), text)
}

/// A trust store failure: the reported failure when it has a code, else the sentence alone.
pub(crate) fn store(e: &TrustError) -> Result<Reported, String> {
    let text = match e {
        TrustError::Revocation(inner) => return Ok(revocation(inner)),
        TrustError::NotAHash(_)
        | TrustError::NamespaceBound { .. }
        | TrustError::ReservedNamespace { .. }
        | TrustError::NotPinned(_)
        | TrustError::Revoked(_)
        | TrustError::TooLarge { .. } => e.to_string(),
        _ => withheld(e.code().unwrap_or(STORE_FAILED)),
    };
    match e.code() {
        Some(code) => Ok(Reported::new(code, text)),
        None => Err(text),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_fallback_variant_shows_its_own_code_beside_the_withheld_sentence() {
        let e = SyncError::Store("failed at /home/someone/secret".to_owned());
        let shown = sync(Path::new("/project"), &e).to_string();
        assert_eq!(
            shown,
            "sync-failed: details are withheld because they can name paths outside the project"
        );
    }

    #[test]
    fn the_withheld_sentence_always_carries_the_code_it_was_given() {
        assert!(withheld("some-code").starts_with("some-code: "));
    }

    /// Every source file under `dir`, as its path and text.
    fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push((
                    path.to_string_lossy().into_owned(),
                    fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }

    #[test]
    fn no_code_outside_the_renderer_builds_the_withheld_sentence() {
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let offenders: Vec<&str> = files
            .iter()
            .filter(|(path, text)| {
                !path.ends_with("/cause.rs")
                    && !path.ends_with("/messages/cli.rs")
                    && text.contains("CliText::Withheld")
            })
            .map(|(path, _)| path.as_str())
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}
