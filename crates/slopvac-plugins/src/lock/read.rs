//! Read `.slopvac/plugins.lock` of a project.

#[cfg(feature = "test-support")]
use std::cell::RefCell;
use std::path::Path;
#[cfg(feature = "test-support")]
use std::rc::Rc;

use slopvac_fs::{InputKind, ReadOptions, read_capped_regular_to_string};
use thiserror::Error;

use super::{LockError, Lockfile, MAX_LOCK_BYTES};
use crate::messages::cli::{Redacted, RunError};

/// A failure to read the lock as a command reports it: its stable code, and a text that names
/// no path and repeats no text of the file.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct LockFailure(RunError);

impl LockFailure {
    /// The stable code of the failure: `plugins-lock-unreadable` or `plugins-lock-invalid`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.0.code()
    }
}

/// Why the lock of a project could not be read. The text comes from the plugins catalog, names no
/// path and repeats no text of the file.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LockReadError {
    /// The file cannot be read, or is missing and the lock is required. The code is
    /// `plugins-lock-unreadable`.
    #[error(transparent)]
    Unreadable(LockFailure),
    /// The file is not a valid lock. The code is `plugins-lock-invalid`.
    #[error(transparent)]
    Invalid(LockFailure),
}

/// The result of reading the lock of a project.
pub type Result<T> = std::result::Result<T, LockReadError>;

impl LockReadError {
    /// The stable code of the failure: `plugins-lock-unreadable` or `plugins-lock-invalid`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreadable(f) | Self::Invalid(f) => f.code(),
        }
    }

    /// The failure as the error of a command, for the command line to report.
    pub(crate) fn into_run_error(self) -> RunError {
        match self {
            Self::Unreadable(f) | Self::Invalid(f) => f.0,
        }
    }
}

#[cfg(feature = "test-support")]
/// What a test hung on the lock reader of this thread: a call as each read starts, and one after
/// each read ends.
#[cfg(feature = "test-support")]
struct Observer {
    on_entry: Rc<dyn Fn()>,
    after_read: Rc<dyn Fn()>,
}

#[cfg(feature = "test-support")]
thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) };
}

/// Ends the observation that [`observe_lock_reads`] started, when dropped.
#[cfg(feature = "test-support")]
#[doc(hidden)]
#[must_use]
pub struct LockReadObservation(std::marker::PhantomData<*const ()>);

#[cfg(feature = "test-support")]
impl Drop for LockReadObservation {
    fn drop(&mut self) {
        OBSERVER.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Call `on_entry` as every read of a lock file on this thread starts, before any I/O and
/// whether or not the read is later refused, and `after_read` once the read has ended, however
/// the read is reached, until the returned guard is dropped. A test counts the reads of one
/// command with the first and changes the file between two of them with the second. The guard
/// is not `Send`: it ends the observation of the thread that made it.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn observe_lock_reads(
    on_entry: impl Fn() + 'static,
    after_read: impl Fn() + 'static,
) -> LockReadObservation {
    OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(Observer {
            on_entry: Rc::new(on_entry),
            after_read: Rc::new(after_read),
        });
    });
    LockReadObservation(std::marker::PhantomData)
}

/// Read and parse `<project_root>/.slopvac/plugins.lock`, refusing a file over
/// [`MAX_LOCK_BYTES`]. A missing file is an empty lock unless `required`.
///
/// # Errors
/// [`LockReadError`] with the code `plugins-lock-unreadable` when the file cannot be read (or is
/// missing and `required`), and `plugins-lock-invalid` when it is not a valid lock. The cause of
/// the second keeps the location only, never the rejected text.
pub fn read_project_lock(project_root: &Path, required: bool) -> Result<Lockfile> {
    #[cfg(feature = "test-support")]
    let hooks = OBSERVER.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|o| (Rc::clone(&o.on_entry), Rc::clone(&o.after_read)))
    });
    #[cfg(feature = "test-support")]
    if let Some((on_entry, _)) = &hooks {
        on_entry();
    }
    let read = read_lock_file(project_root, required);
    #[cfg(feature = "test-support")]
    if let Some((_, after_read)) = &hooks {
        after_read();
    }
    read
}

fn read_lock_file(project_root: &Path, required: bool) -> Result<Lockfile> {
    let path = project_root.join(".slopvac/plugins.lock");
    let invalid = |source: LockError| {
        let detail = source.diagnostic();
        LockReadError::Unreadable(LockFailure(RunError::LockInvalid {
            // The parser's own error can echo the rejected value, so the cause keeps only the
            // location.
            source: Redacted::new(detail.clone()),
            detail,
        }))
    };

    match read_capped_regular_to_string(&path, MAX_LOCK_BYTES as u64, ReadOptions::default()) {
        Ok(text) => Lockfile::parse(&text).map_err(invalid),
        Err(e) if !required && e.input_kind() == Some(InputKind::NotFound) => {
            Lockfile::new(Vec::new()).map_err(invalid)
        }
        Err(source) => Err(LockReadError::Unreadable(LockFailure(
            RunError::LockUnreadable {
                detail: source.to_string(),
                source,
            },
        ))),
    }
}
