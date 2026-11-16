//! Reads `docs/site.toml` at build time so rule links share the docs address with the docs
//! pipeline. The file may be absent (a crate built outside the repository); links are then off.

use std::path::Path;

fn main() -> Result<(), std::env::VarError> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")?;
    let path = Path::new(&manifest).join("../../docs/site.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "site_url" {
            let url = value.trim().trim_matches('"');
            assert!(
                url.starts_with("https://") && !url.contains(['"', '\n']),
                "docs/site.toml: site_url must be an https URL, got {url:?}"
            );
            println!("cargo:rustc-env=SLOPVAC_DOCS_BASE={url}");
        }
    }
    Ok(())
}
