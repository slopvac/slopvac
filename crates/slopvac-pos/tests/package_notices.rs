//! The licence notices the crate redistributes must exist and must ship in the
//! published package.

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn notices_exist_and_are_in_the_package_allowlist() {
    let root = env!("CARGO_MANIFEST_DIR");
    for (file, must_contain) in [
        ("LICENSE-prose", "Copyright (c) 2017-2026 Joseph Kato"),
        ("model/LICENSE-MODEL.md", "CC BY-SA 4.0"),
    ] {
        let text = std::fs::read_to_string(format!("{root}/{file}")).unwrap();
        assert!(text.contains(must_contain), "{file} lacks {must_contain:?}");
        assert!(
            MANIFEST.contains(&format!("\"{file}\"")),
            "{file} is not in the package `include` list"
        );
    }
    assert!(MANIFEST.contains("\"model/en.bin\""));
}
