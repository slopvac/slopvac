//! Capped reads and atomic writes.

use std::fs::{self, File, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::{Error, Result};

/// Read all of `reader`, failing when it holds more than `cap` bytes.
///
/// Memory stays bounded by `cap + 1` bytes, so an endless reader cannot exhaust it.
pub fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(Error::Read)?;
    if buf.len() as u64 > cap {
        return Err(Error::CapExceeded { cap });
    }
    Ok(buf)
}

/// Options for [`write_atomic`].
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Permission bits for a file that does not exist yet. `None` keeps the owner-only `0o600`
    /// that `tempfile` creates. An existing file keeps its own permissions either way.
    pub new_file_mode: Option<u32>,
    /// Create missing parent directories first.
    pub create_parents: bool,
}

/// Replace `path` with `bytes` so a reader sees the old content or the new, never a mix.
///
/// The bytes go to a temporary file in the same directory. The file is synced, renamed over
/// `path`, and then the directory is synced so the rename survives a crash. An existing target
/// keeps its permission bits. The temporary file is removed on failure.
pub fn write_atomic(path: &Path, bytes: &[u8], opts: WriteOptions) -> Result<()> {
    let io = |op, source| Error::Io {
        op,
        path: path.to_path_buf(),
        source,
    };
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if opts.create_parents {
        fs::create_dir_all(dir).map_err(|e| io("create directory for", e))?;
    }
    let mut tmp = tempfile::Builder::new()
        .prefix(".slopvac-tmp-")
        .tempfile_in(dir)
        .map_err(|e| io("create temporary file for", e))?;
    tmp.write_all(bytes).map_err(|e| io("write", e))?;
    let perms = match fs::metadata(path) {
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            opts.new_file_mode.map(Permissions::from_mode)
        }
        Err(e) => return Err(io("stat", e)),
    };
    if let Some(perms) = perms {
        tmp.as_file()
            .set_permissions(perms)
            .map_err(|e| io("set permissions of", e))?;
    }
    tmp.as_file().sync_all().map_err(|e| io("sync", e))?;
    tmp.persist(path).map_err(|e| io("replace", e.error))?;
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| io("sync directory of", e))
}

#[cfg(test)]
mod tests {
    use std::io::repeat;

    use super::*;

    #[test]
    fn read_capped_accepts_exactly_the_cap() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
    }

    #[test]
    fn read_capped_rejects_one_over() {
        assert!(matches!(
            read_capped(&b"abcd"[..], 3),
            Err(Error::CapExceeded { cap: 3 })
        ));
    }

    #[test]
    fn read_capped_stops_an_endless_reader() {
        assert!(matches!(
            read_capped(repeat(b'x'), 1024),
            Err(Error::CapExceeded { .. })
        ));
    }

    #[test]
    fn read_capped_zero_cap_accepts_empty_only() {
        assert_eq!(read_capped(&b""[..], 0).unwrap(), Vec::<u8>::new());
        assert!(read_capped(&b"x"[..], 0).is_err());
    }

    #[test]
    fn write_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        write_atomic(&p, b"one", WriteOptions::default()).unwrap();
        write_atomic(&p, b"two", WriteOptions::default()).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary file is left behind");
    }

    #[test]
    fn write_keeps_existing_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        fs::write(&p, b"old").unwrap();
        fs::set_permissions(&p, Permissions::from_mode(0o640)).unwrap();
        let opts = WriteOptions {
            new_file_mode: Some(0o600),
            ..WriteOptions::default()
        };
        write_atomic(&p, b"new", opts).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn write_new_file_modes() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("a");
        write_atomic(&private, b"x", WriteOptions::default()).unwrap();
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let shared = dir.path().join("b");
        let opts = WriteOptions {
            new_file_mode: Some(0o644),
            ..WriteOptions::default()
        };
        write_atomic(&shared, b"x", opts).unwrap();
        assert_eq!(
            fs::metadata(&shared).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn write_creates_parents_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x/y/f");
        assert!(write_atomic(&p, b"1", WriteOptions::default()).is_err());
        let opts = WriteOptions {
            create_parents: true,
            ..WriteOptions::default()
        };
        write_atomic(&p, b"1", opts).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"1");
    }

    #[test]
    fn failed_write_leaves_the_target_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("target");
        fs::create_dir(&p).unwrap();
        assert!(write_atomic(&p, b"x", WriteOptions::default()).is_err());
        assert!(p.is_dir());
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "temporary file cleaned up"
        );
    }
}
