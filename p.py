p = "crates/slopvac-update/src/notice.rs"
s = open(p).read()
old = s[
    s.index("    /// Store an entry. The caller may ignore the error.") : s.index(
        "    /// Path of the cache file."
    )
]
new = """    /// Store an entry. The caller may ignore the error.
    pub fn write(&self, entry: &Entry) -> std::io::Result<()> {
        self.write_with(entry, |tmp| {
            tmp.persist(&self.path)
                .map(|_| ())
                .map_err(|e| (e.error, e.file))
        })
    }

    /// [`Cache::write`] with the rename step supplied, so a test can make it fail.
    ///
    /// This is the only place that retries the rename: `write` calls it, so a test that
    /// injects a failing step here fails when the retry is removed.
    fn write_with<F>(&self, entry: &Entry, persist: F) -> std::io::Result<()>
    where
        F: FnMut(tempfile::NamedTempFile) -> Result<(), (std::io::Error, tempfile::NamedTempFile)>,
    {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        // A unique temp file per writer, renamed over the target: readers see the old or the new
        // entry, never a mix, and writers never share a file.
        let mut tmp = tempfile::Builder::new()
            .prefix(".update-check-")
            .tempfile_in(dir)?;
        std::io::Write::write_all(
            &mut tmp,
            &serde_json::to_vec(entry).map_err(std::io::Error::other)?,
        )?;
        persist_retrying(tmp, persist, PERSIST_RETRIES, PERSIST_PAUSE)
    }

"""
s = s.replace(old, new)
s = s.replace(
    """    #[test]
    fn overlapping_writers_never_corrupt""",
    """    /// Fail the rename `failures` times with `PermissionDenied`, then do the real rename.
    fn flaky_persist(
        target: &Path,
        failures: u32,
        calls: &std::cell::Cell<u32>,
    ) -> impl FnMut(
        tempfile::NamedTempFile,
    ) -> Result<(), (std::io::Error, tempfile::NamedTempFile)> {
        let target = target.to_path_buf();
        move |tmp| {
            calls.set(calls.get() + 1);
            if calls.get() <= failures {
                Err((denied(), tmp))
            } else {
                tmp.persist(&target).map(|_| ()).map_err(|e| (e.error, e.file))
            }
        }
    }

    #[test]
    fn cache_write_retries_a_transient_permission_denied_and_stores_the_entry() {
        let (_dir, cache) = cache();
        let calls = std::cell::Cell::new(0);
        cache
            .write_with(&entry(7, "9.9.9"), flaky_persist(cache.path(), 3, &calls))
            .expect("the write succeeds after the retries");
        assert_eq!(calls.get(), 4, "three refusals and one success");
        assert_eq!(cache.read().expect("stored").latest, "9.9.9");
    }

    #[test]
    fn cache_write_gives_up_after_its_retry_budget_and_leaves_no_temp_file() {
        let (dir, cache) = cache();
        let calls = std::cell::Cell::new(0);
        let err = cache
            .write_with(
                &entry(7, "9.9.9"),
                flaky_persist(cache.path(), u32::MAX, &calls),
            )
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(calls.get(), PERSIST_RETRIES + 1, "one try plus the retries");
        assert!(cache.read().is_none());
        assert!(dir_is_empty(&dir.path().join("c")), "the temp file is cleaned up");
    }

    #[test]
    fn overlapping_writers_never_corrupt""",
    1,
)
open(p, "w").write(s)
