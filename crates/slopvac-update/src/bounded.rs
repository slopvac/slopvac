//! Bounded reads of files under user-writable directories.
//!
//! The update cache, the install receipt and the wheel metadata sit on paths a user or another
//! program controls, so each can be a FIFO, a directory or a link to a device. These reads open
//! without blocking, check that the open handle is a regular file, and stop at a cap.

use std::io::Read as _;
use std::path::Path;

/// The bytes of a regular file of at most `cap` bytes, else `None`.
pub(crate) fn read_regular(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO with no writer would block an ordinary open.
        opts.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or_default());
    }
    let handle = opts.open(path).ok()?;
    if !handle.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    handle.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

/// Like [`read_regular`], as UTF-8 text.
pub(crate) fn read_text(path: &Path, cap: u64) -> Option<String> {
    String::from_utf8(read_regular(path, cap)?).ok()
}
