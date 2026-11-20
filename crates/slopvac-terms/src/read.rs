//! Size-capped reads of terms, blocklist and document files.

use std::path::Path;

use crate::TermsError;

/// The most bytes of one terms or blocklist file. The built-in terms file is about 30 KiB, so
/// 1 MiB leaves a wide margin.
pub const MAX_TERMS_FILE_BYTES: u64 = 1024 * 1024;

/// Why a capped read failed.
pub(crate) enum ReadFailure {
    /// The file does not exist. Carries the OS message.
    Missing(String),
    /// Any other failure, ready to return.
    Error(TermsError),
}

/// The text of an I/O error without any path a library may have put in it: the OS error, or
/// the kind.
pub(crate) fn scrub(e: &std::io::Error) -> String {
    e.raw_os_error().map_or_else(
        || e.kind().to_string(),
        |code| std::io::Error::from_raw_os_error(code).to_string(),
    )
}

/// Read `path` as UTF-8 text of at most `cap` bytes. `shown` names the file in errors. A file
/// over `cap` fails with [`TermsError::TooLarge`].
pub(crate) fn read(path: &Path, shown: &str, cap: u64) -> Result<String, ReadFailure> {
    // The reader refuses a symlink, but a symlinked terms file is supported. Resolve the link
    // first; a link to a FIFO or device then fails as not regular, without blocking.
    // A missing file or a dangling link is "missing", as it was before this read.
    let resolved = match std::fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReadFailure::Missing(scrub(&e)));
        }
        Err(_) => path.to_path_buf(),
    };
    slopvac_fs::read_capped_regular_to_string(&resolved, cap, slopvac_fs::ReadOptions::new())
        .map_err(|e| match e {
            slopvac_fs::Error::Input {
                kind: slopvac_fs::InputKind::TooLarge,
                ..
            } => ReadFailure::Error(TermsError::TooLarge {
                code: if cap == slopvac_fs::MAX_DOCUMENT_BYTES {
                    "document-too-large"
                } else {
                    "terms-file-too-large"
                },
                path: shown.to_owned(),
                cap,
            }),
            slopvac_fs::Error::Input {
                kind: slopvac_fs::InputKind::NotFound,
                ..
            } => ReadFailure::Missing(scrub(&std::io::ErrorKind::NotFound.into())),
            slopvac_fs::Error::Input {
                source: Some(source),
                ..
            } => ReadFailure::Error(TermsError::Io {
                path: shown.to_owned(),
                message: scrub(&source),
            }),
            other => ReadFailure::Error(TermsError::Io {
                path: shown.to_owned(),
                message: other.to_string(),
            }),
        })
}

/// Like [`read`], with a missing file reported as an I/O error that names `shown`.
pub(crate) fn read_existing(path: &Path, shown: &str, cap: u64) -> Result<String, TermsError> {
    read(path, shown, cap).map_err(|f| match f {
        ReadFailure::Missing(message) => TermsError::Io {
            path: shown.to_owned(),
            message,
        },
        ReadFailure::Error(e) => e,
    })
}
