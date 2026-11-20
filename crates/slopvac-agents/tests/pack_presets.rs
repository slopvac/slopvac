use std::fs;
use std::path::Path;
use slopvac_config::{Inputs, load};
use slopvac_packs::Packs;

#[test]
fn probe() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::write(root.join("slopvac.toml"), "schema_version = 3\nstyles = [\"ste\"]\n\n[[genres]]\ngenre = \"tutorial\"\nfiles = [\"**\"]\n").unwrap();
    let packs = Packs::core().unwrap();
    let cfg = load(&root.join("slopvac.toml"), &packs).unwrap();
    let r = cfg.resolve(Path::new("a.md"), &Inputs { presets: &packs, front_matter: &[], directives: &[], cli: &[], env: &[] });
    println!("{}", serde_json::to_string_pretty(&r.explain().iter().map(|e| format!("{e:?}")).collect::<Vec<_>>()).unwrap());
}
