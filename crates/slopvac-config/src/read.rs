//! Read a config file: regular files only, bounded, and never blocking.
//!
//! Config discovery walks up from the working directory and loads a file the user did not name,
//! so it must not hang on a FIFO, stream a device or read without a limit. The check runs on the
//! opened handle, so a file swapped in after a name check cannot slip through.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;

/// Largest config file, extended or not.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The text of the file at `path`.
///
/// # Errors
/// [`io::ErrorKind::InvalidInput`] for a directory, FIFO or device,
/// [`io::ErrorKind::FileTooLarge`] over [`MAX_CONFIG_BYTES`], [`io::ErrorKind::InvalidData`] for
/// invalid UTF-8, and the OS error otherwise.
pub fn read_text(path: &Path) -> io::Result<String> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO with no writer would block an ordinary open.
        opts.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or_default());
    }
    let file = opts.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "larger than the config limit",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.utf8_error().to_string()))
}
