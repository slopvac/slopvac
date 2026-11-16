//! Bounded reads of files that the user did not hand to slopvac on purpose.
//!
//! Doctor opens project files (`slopvac.toml`, the lockfile) and cache files on paths it
//! discovers. A path can be a directory, a FIFO or a device, and a file can be huge, so every read
//! opens without blocking, checks the open handle is a regular file and stops at a cap.

use std::io::Read;
use std::path::Path;

/// Why a bounded read did not return text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadProblem {
    /// A directory, FIFO, device or other non-regular file.
    NotRegular,
    /// Over the cap.
    TooLarge,
    /// Any other I/O failure, including invalid UTF-8; carries the failure's kind.
    Io(std::io::ErrorKind),
}

/// The text of `path`, or `None` when it does not exist.
pub fn read_bounded(path: &Path, cap: u64) -> Result<Option<String>, ReadProblem> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO with no writer would block an ordinary open.
        opts.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or_default());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS lets a directory open, so the handle check names it.
        opts.custom_flags(0x0200_0000);
    }
    let handle = match opts.open(path) {
        Ok(h) => h,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(ReadProblem::Io(e.kind())),
    };
    if !handle
        .metadata()
        .map_err(|e| ReadProblem::Io(e.kind()))?
        .is_file()
    {
        return Err(ReadProblem::NotRegular);
    }
    let mut text = String::new();
    handle
        .take(cap + 1)
        .read_to_string(&mut text)
        .map_err(|e| ReadProblem::Io(e.kind()))?;
    if !u64::try_from(text.len()).is_ok_and(|n| n <= cap) {
        return Err(ReadProblem::TooLarge);
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_is_none_and_text_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_bounded(&dir.path().join("a"), 10), Ok(None));
        std::fs::write(dir.path().join("a"), "hello").unwrap();
        assert_eq!(
            read_bounded(&dir.path().join("a"), 10),
            Ok(Some("hello".into()))
        );
    }

    #[test]
    fn the_cap_is_inclusive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), "12345").unwrap();
        assert!(read_bounded(&dir.path().join("a"), 5).is_ok());
        assert_eq!(
            read_bounded(&dir.path().join("a"), 4),
            Err(ReadProblem::TooLarge)
        );
    }

    #[test]
    fn a_directory_is_not_regular() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_bounded(dir.path(), 10), Err(ReadProblem::NotRegular));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_not_regular_and_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("f");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(read_bounded(&fifo, 10), Err(ReadProblem::NotRegular));
    }

    #[test]
    fn invalid_utf8_is_an_io_problem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), [0xff, 0xfe]).unwrap();
        assert_eq!(
            read_bounded(&dir.path().join("a"), 10),
            Err(ReadProblem::Io(std::io::ErrorKind::InvalidData))
        );
    }
}
