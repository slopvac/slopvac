//! Failures of library operations as the text and code that `slopvac plugins` reports.
//!
//! Each function keeps the sentence of the variants whose fields are typed and path free, shows
//! a typed path under the project root (`<external>/<file name>` outside it), and renders any
//! other variant as the withheld text: its free text can name a local path, and the code still
//! tells the user which failure it was.

use std::path::Path;

use slopvac_fs::display_under_root;
use slopvac_messages::{Catalog, Entry, render};

use crate::messages::cli::Reported;
use crate::lock::LockDiff;
use crate::messages::sync::{cache_tampered, config_drift, update_busy};
use crate::revoke::RevocationError;
use crate::sync::SyncError;
use crate::trust::TrustError;
use crate::update::UpdateError;

/// The CI refusal when the configured plugin ids and the lock disagree. It keeps the
/// `lock-mismatch` code and says that the config was compared, not the resolver output.
pub(crate) fn config_lock_drift(drift: LockDiff) -> Reported {
    let text = config_drift(&drift);
    Reported::new(SyncError::LockMismatch(drift).code(), text)
}

/// The one sentence that stands in for withheld detail. It is private, so the only way to render
/// it is [`withheld`], which always puts the code of the failure beside it.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
enum WithheldText {
    /// Stands in for failure text that can name a local path.
    #[code("plugins-detail-withheld")]
    #[text("{code}: details are withheld because they can name paths outside the project")]
    Withheld { code: String },
}

/// The registered entry of the withheld sentence, for the group's message registry.
pub(crate) fn entries() -> &'static [Entry] {
    WithheldText::entries()
}

/// The sentence that stands in for withheld detail, with the code of the failure it replaces.
/// Every withheld text goes through here, so the code always shows beside it.
pub(crate) fn withheld(code: &str) -> String {
    render(&WithheldText::Withheld {
        code: code.to_owned(),
    })
}

/// The code that the spawn path reports for a trust failure that has none of its own, so that
/// `status` and `trust` report the same code the host reports at spawn.
fn spawn_code(e: &TrustError) -> Option<&'static str> {
    match e {
        TrustError::Program(_) | TrustError::Environment(_) => Some("plugin-usage"),
        TrustError::KeyMissing { .. } => Some("trust-key-missing"),
        TrustError::KeyMismatch => Some("trust-key-mismatch"),
        TrustError::KeyUnusable { .. } => Some("trust-key-unusable"),
        _ => None,
    }
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
/// A refused program or environment, and an unusable trust key, report the code of the spawn
/// path. A failure of the store itself has no code of its own and reports `plugins-store-failed`.
pub(crate) fn store(e: &TrustError) -> Result<Reported, String> {
    let text = match e {
        TrustError::Revocation(inner) => return Ok(revocation(inner)),
        TrustError::NotAHash(_)
        | TrustError::NamespaceBound { .. }
        | TrustError::ReservedNamespace { .. }
        | TrustError::NotPinned(_)
        | TrustError::Revoked(_)
        | TrustError::TooLarge { .. } => e.to_string(),
        _ => withheld(e.code().or_else(|| spawn_code(e)).unwrap_or(STORE_FAILED)),
    };
    match e.code().or_else(|| spawn_code(e)) {
        Some(code) => Ok(Reported::new(code, text)),
        None => Err(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_refused_program_reports_the_spawn_code_not_the_store_code() {
        let e = TrustError::Program("/home/someone/prog".to_owned());
        let r = store(&e).unwrap();
        assert_eq!(slopvac_messages::Catalog::code(&r), "plugin-usage");
        assert_eq!(
            r.to_string(),
            "plugin-usage: details are withheld because they can name paths outside the project"
        );
    }

    #[test]
    fn each_key_failure_reports_its_own_spawn_code() {
        for (e, code) in [
            (
                TrustError::KeyMissing {
                    file: "k".to_owned(),
                },
                "trust-key-missing",
            ),
            (TrustError::KeyMismatch, "trust-key-mismatch"),
            (
                TrustError::KeyUnusable {
                    file: "k".to_owned(),
                    why: "w".to_owned(),
                },
                "trust-key-unusable",
            ),
        ] {
            let r = store(&e).unwrap();
            assert_eq!(slopvac_messages::Catalog::code(&r), code);
        }
    }

    #[test]
    fn a_failure_of_the_store_itself_has_only_the_store_code() {
        let e = TrustError::Corrupt {
            path: "/x/trust.json".into(),
            why: "w".to_owned(),
        };
        assert_eq!(
            store(&e).unwrap_err(),
            "plugins-store-failed: details are withheld because they can name paths outside the project"
        );
    }
}
