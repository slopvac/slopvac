//! A `slopvac.toml` file and a document's `slopvac:` front-matter block must load or be
//! rejected with an error, never panic.

#![no_main]

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use slopvac_config::frontmatter::{Policy, read};
use slopvac_config::{NoPresets, load};

/// The config file path inside a private directory that this process created.
///
/// `tempfile` creates the directory atomically with an unpredictable name and mode 0700, so
/// no other user can pre-create it or plant a symlink in it. libFuzzer ends the process with
/// `exit`, which skips destructors, so the file is removed after every input and only the
/// empty directory is left for the OS to clean from the temp directory.
fn scratch_file() -> &'static PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let dir = tempfile::Builder::new()
            .prefix("slopvac-fuzz-config-")
            .tempdir()
            .expect("create the private scratch directory");
        dir.keep().join("slopvac.toml")
    })
}

fuzz_target!(|text: &str| {
    let path = scratch_file();
    fs::write(path, text).expect("write the scratch config");
    let _ = load(path, &NoPresets);
    fs::remove_file(path).expect("remove the scratch config");
    let _ = read(text, "doc.md", &Policy::core());
});
