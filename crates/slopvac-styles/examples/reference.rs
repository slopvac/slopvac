//! Write the style reference pages under a directory.
//!
//! `cargo run -p slopvac-styles --example reference -- <out-dir>` writes
//! `styles/index.md` and one page per bundled preset below `<out-dir>`.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| std::io::Error::other("usage: reference <out-dir>"))?,
    );
    for (path, text) in slopvac_styles::reference_pages() {
        let file = out.join(path);
        let dir = file
            .parent()
            .ok_or_else(|| std::io::Error::other("page paths have a directory"))?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(file, text)?;
    }
    Ok(())
}
