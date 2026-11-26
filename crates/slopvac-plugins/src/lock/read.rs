//! Read `.slopvac/plugins.lock` of a project.

use std::cell::RefCell;
use std::fmt;
use std::path::Path;
use std::rc::Rc;

use slopvac_fs::{InputKind, ReadOptions, read_capped_regular_to_string};

use super::{LockError, Lockfile, MAX_LOCK_BYTES};
use crate::messages::cli::{Redacted, RunError};

/// Why the lock of a project could not be read: the file cannot be read, or it is not a valid
/// lock. The text comes from the plugins catalog, names no path and repeats no text of the file.
#[derive(Debug)]
pub struct LockReadError(RunError);

impl LockReadError {
    /// The stable code of the failure: `plugins-lock-unreadable` or `plugins-lock-invalid`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.0.code()
    }

    /// The failure as the error of a command, for the command line to report.
    pub(crate) fn into_run_error(self) -> RunError {
        self.0
    }
}

impl fmt::Display for LockReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for LockReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0)
    }
}

thread_local! {
    /// What a test hung on the lock reader of this thread.
    static OBSERVER: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Ends the observation that [`observe_lock_reads`] started, when dropped.
#[doc(hidden)]
#[must_use]
pub struct LockReadObservation(());

impl Drop for LockReadObservation {
    fn drop(&mut self) {
        OBSERVER.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Call `observer` after every read of a lock file on this thread, however the read is reached,
/// until the returned guard is dropped. A test counts the reads of one command with it, and
/// changes the file between two of them.
#[doc(hidden)]
pub fn observe_lock_reads(observer: impl Fn() + 'static) -> LockReadObservation {
    OBSERVER.with(|slot| *slot.borrow_mut() = Some(Rc::new(observer)));
    LockReadObservation(())
}

/// Read and parse `<project_root>/.slopvac/plugins.lock`, refusing a file over
/// [`MAX_LOCK_BYTES`]. A missing file is an empty lock unless `required`.
///
/// # Errors
/// [`LockReadError`] with the code `plugins-lock-unreadable` when the file cannot be read (or is
/// missing and `required`), and `plugins-lock-invalid` when it is not a valid lock. The cause of
/// the second keeps the location only, never the rejected text.
pub fn read_project_lock(project_root: &Path, required: bool) -> Result<Lockfile, LockReadError> {
    let read = read_lock_file(project_root, required);
    let observer = OBSERVER.with(|slot| slot.borrow().clone());
    if let Some(observer) = observer {
        observer();
    }
    read
}

fn read_lock_file(project_root: &Path, required: bool) -> Result<Lockfile, LockReadError> {
    let path = project_root.join(".slopvac/plugins.lock");
    let invalid = |source: LockError| {
        let detail = source.diagnostic();
        LockReadError(RunError::LockInvalid {
            // The parser's own error can echo the rejected value, so the cause keeps only the
            // location.
            source: Redacted::new(detail.clone()),
            detail,
        })
    };

    match read_capped_regular_to_string(&path, MAX_LOCK_BYTES as u64, ReadOptions::default()) {
        Ok(text) => Lockfile::parse(&text).map_err(invalid),
        Err(e) if !required && e.input_kind() == Some(InputKind::NotFound) => {
            Lockfile::new(Vec::new()).map_err(invalid)
        }
        Err(source) => Err(LockReadError(RunError::LockUnreadable {
            detail: source.to_string(),
            source,
        })),
    }
}
