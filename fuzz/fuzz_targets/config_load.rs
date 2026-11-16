//! A `slopvac.toml` file and a document's `slopvac:` front-matter block must load or be
//! rejected with an error, never panic.

#![no_main]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use slopvac_config::frontmatter::{Policy, read};
use slopvac_config::{NoPresets, load};

/// The config file path inside a directory that this process created.
///
/// `tempfile` creates the directory atomically under an unpredictable name. On unix the
/// builder asks for mode 0700 in the same `mkdir` call, so a permissive umask cannot widen
/// it and no other user can plant a symlink in it. The assert below fails the run if that
/// ever stops being true (`rust-fuzz.yml` runs this target under `umask 000`). libFuzzer
/// ends the process with `exit`, which skips destructors, so the file is removed after
/// every input and only the empty directory is left for the OS to clean from the temp
/// directory.
fn scratch_file() -> &'static PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let mut builder = tempfile::Builder::new();
        builder.prefix("slopvac-fuzz-config-");
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let dir = builder
            .tempdir()
            .expect("create the private scratch directory")
            .keep();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir)
                .expect("stat the scratch directory")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "the scratch directory is not owner-only"
            );
        }
        dir.join("slopvac.toml")
    })
}

fuzz_target!(|text: &str| {
    let path = scratch_file();
    // `create_new` fails on any existing entry, a symlink included, and never follows one.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create the scratch config");
    file.write_all(text.as_bytes())
        .expect("write the scratch config");
    drop(file);
    let _ = load(path, &NoPresets);
    fs::remove_file(path).expect("remove the scratch config");
    let _ = read(text, "doc.md", &Policy::core());
});
