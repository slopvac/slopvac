//! Size-capped file reads for configuration and documents.
//!
//! Every file the crate reads goes through [`read_text`], so no input can make a run allocate
//! more than its cap. The caps and their error codes are listed in `docs/contract/policies.md`.

use std::io;
use std::path::Path;

/// The most bytes of one config file: `slopvac.toml` and every file it extends. The largest
/// shipped rule file is about 130 KiB, so 1 MiB leaves a wide margin for a config.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The deepest `extends` nesting: the nearest config extends a file that extends a file, and
/// so on. A real chain is two or three levels, so 32 leaves a wide margin.
pub const MAX_EXTENDS_DEPTH: usize = 32;

/// The most config files one `extends` graph may expand to, counting a file once for each time
/// it applies. A real graph has fewer than ten, so 256 leaves a wide margin.
pub const MAX_EXTENDS_FILES: usize = 256;

/// The most bytes read in total from the files of one `extends` graph, counting each file once.
/// A real graph is under 100 KiB, so 8 MiB leaves a wide margin.
pub const MAX_EXTENDS_BYTES: u64 = 8 * 1024 * 1024;

/// The most bytes of config text an `extends` graph may apply, counting a file once for each
/// time it applies. It bounds the merge work and the settings the result holds. A real graph is
/// under 100 KiB, so 64 MiB leaves a wide margin.
pub const MAX_EXTENDS_APPLIED_BYTES: u64 = 64 * 1024 * 1024;

/// The most bytes of one document that the crate reads for front matter and directives.
pub use slopvac_fs::MAX_DOCUMENT_BYTES;

/// Why a capped read failed.
#[derive(Debug)]
pub(crate) enum ReadFailure {
    /// The file is larger than the cap, in bytes.
    TooLarge(u64),
    /// The file could not be opened, read or decoded.
    Io(io::Error),
}

/// Read `path` as UTF-8 text of at most `cap` bytes.
pub(crate) fn read_text(path: &Path, cap: u64) -> Result<String, ReadFailure> {
    slopvac_fs::read_text_capped(path, cap).map_err(|e| match e {
        slopvac_fs::Error::CapExceeded { cap } => ReadFailure::TooLarge(cap),
        slopvac_fs::Error::Io { source, .. } | slopvac_fs::Error::Read(source) => {
            ReadFailure::Io(source)
        }
        slopvac_fs::Error::NotUtf8 { .. } => {
            ReadFailure::Io(io::Error::from(io::ErrorKind::InvalidData))
        }
        other => ReadFailure::Io(io::Error::other(other.to_string())),
    })
}
