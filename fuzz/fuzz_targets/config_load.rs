//! A `slopvac.toml` file and a document's `slopvac:` front-matter block must load or be
//! rejected with an error, never panic.

#![no_main]

use std::fs;
use std::path::PathBuf;

use libfuzzer_sys::fuzz_target;
use slopvac_config::frontmatter::{Policy, read};
use slopvac_config::{NoPresets, load};

fn scratch_file() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-fuzz-config-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create the scratch directory");
    dir.join("slopvac.toml")
}

fuzz_target!(|text: &str| {
    let path = scratch_file();
    fs::write(&path, text).expect("write the scratch config");
    let _ = load(&path, &NoPresets);
    let _ = read(text, "doc.md", &Policy::core());
});
