//! Read `.slopvac/plugins.lock` of a project.

use std::path::Path;

use slopvac_fs::{read_capped_regular_to_string, InputKind, ReadOptions};

use super::{LockError, Lockfile, MAX_LOCK_BYTES};
use crate::messages::cli::{Redacted, RunError};

/// Read and parse `<project_root>/.slopvac/plugins.lock`, refusing a file over
/// [`MAX_LOCK_BYTES`]. A missing file is an empty lock unless `required`.
///
/// # Errors
/// [`RunError::LockUnreadable`] when the file cannot be read (or is missing and `required`),
/// and [`RunError::LockInvalid`] when it is not a valid lock. The cause of the second keeps the
/// location only, never the rejected text.
pub fn read_project_lock(project_root: &Path, required: bool) -> Result<Lockfile, RunError> {
    let path = project_root.join(".slopvac/plugins.lock");
    let invalid = |source: LockError| {
        let detail = source.diagnostic();
        RunError::LockInvalid {
            // The parser's own error can echo the rejected value, so the cause keeps only the
            // location.
            source: Redacted::new(detail.clone()),
            detail,
        }
    };

    match read_capped_regular_to_string(&path, MAX_LOCK_BYTES as u64, ReadOptions::default()) {
        Ok(text) => Lockfile::parse(&text).map_err(invalid),
        Err(e) if !required && e.input_kind() == Some(InputKind::NotFound) => {
            Lockfile::new(Vec::new()).map_err(invalid)
        }
        Err(source) => Err(RunError::LockUnreadable {
            detail: source.to_string(),
            source,
        }),
    }
}
