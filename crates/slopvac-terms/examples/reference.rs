//! Write the terms reference page under a directory.
//!
//! `cargo run -p slopvac-terms --example reference -- <out-dir>` writes
//! `terms/index.md` below `<out-dir>`.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| std::io::Error::other("usage: reference <out-dir>"))?,
    );
    for (path, text) in slopvac_terms::reference_pages() {
        let file = out.join(path);
        let dir = file
            .parent()
            .ok_or_else(|| std::io::Error::other("page paths have a directory"))?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(file, text)?;
    }
    Ok(())
}
