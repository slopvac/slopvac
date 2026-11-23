//! Every access to the 2.x `overrides` list goes through `src/overrides.rs`. That module
//! refuses a list whose entries are not all tables before any rewrite, and it hands the list
//! out to change only with the proof that its check ran. A module that read or changed
//! `overrides` on its own could skip that check and drop an entry it cannot read.

use std::fs;
use std::path::Path;

/// The one module that may name the key.
const OWNER: &str = "overrides.rs";

/// Lines outside the owner that name the key without reading or changing the list, as
/// `(file, trimmed line)`.
const ALLOWED: &[(&str, &str)] = &[
    // The list of top-level keys that 3.0 keeps.
    ("config.rs", "\"overrides\","),
    // The dispatch to `migrate_overrides`, which changes the list through the owner.
    (
        "config.rs",
        "\"overrides\" => migrate_overrides(t, &kp, cx),",
    ),
];

/// The lines of `text`, the source file `name`, that name the `overrides` key and are not
/// allowed.
fn stray_accesses(name: &str, text: &str) -> Vec<String> {
    if name == OWNER {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("\"overrides\""))
        .filter(|(_, line)| !ALLOWED.contains(&(name, line.trim())))
        .map(|(i, line)| format!("{name}:{}: {}", i + 1, line.trim()))
        .collect()
}

fn sources(dir: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            sources(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((name, fs::read_to_string(&path)?));
        }
    }
    Ok(())
}

#[test]
fn no_module_but_the_owner_reads_or_changes_overrides() -> std::io::Result<()> {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    )?;
    assert!(
        files.iter().any(|(name, _)| name == OWNER),
        "the sweep did not find {OWNER}"
    );
    let stray: Vec<String> = files
        .iter()
        .flat_map(|(name, text)| stray_accesses(name, text))
        .collect();
    assert_eq!(stray, Vec::<String>::new());
    Ok(())
}

#[test]
fn the_sweep_flags_a_module_that_changes_overrides_itself() {
    let text = "fn f(doc: &mut DocumentMut) {\n    doc.get_mut(\"overrides\");\n}\n";
    assert_eq!(
        stray_accesses("preserve_doc.rs", text),
        ["preserve_doc.rs:2: doc.get_mut(\"overrides\");"]
    );
}
