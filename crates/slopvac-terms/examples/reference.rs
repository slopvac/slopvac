//! Write the terms reference page under a directory.
//!
//! `cargo run -p slopvac-terms --example reference -- <out-dir>` writes
//! `terms/index.md` below `<out-dir>`.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: reference <out-dir>"));
    for (path, text) in slopvac_terms::reference_pages() {
        let file = out.join(path);
        std::fs::create_dir_all(file.parent().expect("page paths have a directory"))?;
        std::fs::write(file, text)?;
    }
    Ok(())
}
