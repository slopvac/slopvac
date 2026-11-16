//! Write the genre reference pages under a directory.
//!
//! `cargo run -p slopvac-context --example reference -- <out-dir>` writes
//! `genres/index.md` and one page per built-in genre below `<out-dir>`.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: reference <out-dir>"));
    for (path, text) in slopvac_context::reference_pages() {
        let file = out.join(path);
        std::fs::create_dir_all(file.parent().expect("page paths have a directory"))?;
        std::fs::write(file, text)?;
    }
    Ok(())
}
