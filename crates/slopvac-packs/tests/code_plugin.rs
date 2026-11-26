//! A code plugin loads from its installed tree and its lock entry.
//!
//! The tree is written by hand: a manifest and an empty script. Expected values come from the
//! contract (plugins.md section 2, plugin-protocol.md section 4.3), not from the loader.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_packs::{CodePluginContext, CodePluginError, load_code_plugin};
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::lock::{LockedPack, Lockfile, PackId};
use slopvac_plugins::protocol::{FeatureSet, HookDecl, PluginDeclaration};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PATH_ENV: &str = "/usr/bin:/bin";

/// The manifest of a code plugin named `acme` whose `[plugin]` table starts with `command`.
fn manifest(id: &str, version: &str, plugin: &str) -> String {
    format!(
        "schema_version = 3\nid = \"{id}\"\nversion = \"{version}\"\nslopvac_api = 1\nkind = \"code\"\n\n\
         [provides]\n\n[plugin]\n{plugin}"
    )
}

const TABLE: &str = "command = \"bin/acme\"\nargs = [\"--serve\", \"x\"]\nnetwork = true\n\
    hooks = [{ kind = \"tagger\", id = \"t\" }]\n\
    features = { required = [\"document-path\"], optional = [] }\n";

/// A lock entry for `acme` 1.0.0 with the given kind and source line.
fn locked(kind: &str, source_and_signature: &str) -> LockedPack {
    let text = format!(
        "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nkind = \"{kind}\"\nversion = \"1.0.0\"\n\
         slopvac_api = 1\n{source_and_signature}digest = \"{DIGEST}\"\n"
    );
    let lock = Lockfile::parse(&text).unwrap();
    lock.get(&"acme".parse::<PackId>().unwrap())
        .unwrap()
        .clone()
}

/// Load with the manifest bytes as the loader's hash pass would hand them over.
fn load_from(
    locked: &LockedPack,
    dir: &Path,
    context: &CodePluginContext,
) -> Result<
    (
        slopvac_plugins::trust::VerifiedPlugin,
        slopvac_plugins::host::PluginSpec,
    ),
    CodePluginError,
> {
    let manifest = fs::read(dir.join("slopvac-plugin.toml")).ok();
    load_code_plugin(locked, dir, manifest.as_deref(), context)
}

fn path_locked() -> LockedPack {
    locked("code", "source = \"plugins/acme\"\n")
}

/// Write `manifest_text` and the script into a fresh tree, and return the context for it.
fn install(root: &Path, manifest_text: &str) -> CodePluginContext {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("slopvac-plugin.toml"), manifest_text).unwrap();
    fs::write(root.join("bin/acme"), "").unwrap();
    CodePluginContext::new(tree_hash(root).unwrap(), PATH_ENV)
}

#[rstest]
#[case::data_plugin("data", manifest("acme", "1.0.0", TABLE), |e: &CodePluginError| matches!(e, CodePluginError::NotCode { .. }))]
#[case::id_differs("code", manifest("evil", "1.0.0", TABLE), |e: &CodePluginError| matches!(e, CodePluginError::Mismatch { field: "id", .. }))]
#[case::version_differs("code", manifest("acme", "2.0.0", TABLE), |e: &CodePluginError| matches!(e, CodePluginError::Mismatch { field: "version", .. }))]
#[case::no_plugin_table(
    "code",
    "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[provides]\n".to_owned(),
    |e: &CodePluginError| matches!(e, CodePluginError::Manifest { .. })
)]
#[case::command_leaves_the_tree(
    "code",
    manifest("acme", "1.0.0", &TABLE.replace("bin/acme", "../x")),
    |e: &CodePluginError| matches!(e, CodePluginError::Command { .. })
)]
#[case::command_with_backslash(
    "code",
    manifest("acme", "1.0.0", &TABLE.replace("bin/acme", "a\\\\b")),
    |e: &CodePluginError| matches!(e, CodePluginError::Command { .. })
)]
#[case::command_with_leading_dash(
    "code",
    manifest("acme", "1.0.0", &TABLE.replace("bin/acme", "-x")),
    |e: &CodePluginError| matches!(e, CodePluginError::Command { .. })
)]
fn a_plugin_that_breaks_the_contract_is_refused(
    #[case] lock_kind: &str,
    #[case] manifest_text: String,
    #[case] expected: fn(&CodePluginError) -> bool,
) {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest_text);
    let lock = locked(lock_kind, "source = \"plugins/acme\"\n");
    let error = load_from(&lock, dir.path(), &context).unwrap_err();
    assert!(expected(&error), "wrong refusal: {error:?}");
}

#[test]
fn a_native_plugin_runs_its_command_from_the_tree_root() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("acme", "1.0.0", TABLE));
    let (plugin, spec) = load_from(&path_locked(), dir.path(), &context).unwrap();
    assert_eq!(spec.program, dir.path().join("bin/acme"));
    assert_eq!(spec.args, ["--serve", "x"]);
    assert_eq!(spec.cwd, dir.path());
    assert_eq!(spec.path_env, PATH_ENV);
    assert_eq!(spec.env, Vec::<(String, String)>::new());
    assert_eq!(plugin.root, dir.path());
}

#[test]
fn the_declaration_comes_from_the_plugin_table() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("acme", "1.0.0", TABLE));
    let (_, spec) = load_from(&path_locked(), dir.path(), &context).unwrap();
    assert_eq!(
        spec.declaration,
        PluginDeclaration {
            network: true,
            hooks: vec![HookDecl {
                kind: "tagger".into(),
                id: "t".into()
            }],
            features: FeatureSet {
                required: vec!["document-path".into()],
                optional: vec![],
            },
        }
    );
}

#[test]
fn a_plugin_takes_its_identity_from_the_lock_and_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("acme", "1.0.0", TABLE));
    let (plugin, _) = load_from(&path_locked(), dir.path(), &context).unwrap();
    assert_eq!(plugin.name, "acme");
    assert_eq!(plugin.version.as_deref(), Some("1.0.0"));
    assert_eq!(plugin.tree, context.tree().clone());
}

#[test]
fn a_signed_registry_plugin_takes_its_digest_and_key_from_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("acme", "1.0.0", TABLE))
        .with_key_fingerprint("ab12".to_owned());
    // The signature table follows the digest line in the lock text.
    let text = format!(
        "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
         slopvac_api = 1\nsource = \"oci://registry.invalid/acme/acme:1.0.0\"\ndigest = \"{DIGEST}\"\n\
         [plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\n"
    );
    let signed = Lockfile::parse(&text)
        .unwrap()
        .get(&"acme".parse::<PackId>().unwrap())
        .unwrap()
        .clone();
    let (plugin, _) = load_from(&signed, dir.path(), &context).unwrap();
    assert_eq!(plugin.manifest_digest.as_deref(), Some(DIGEST));
    assert_eq!(plugin.key_id.as_deref(), Some("A1B2C3D4E5F60718"));
    assert_eq!(plugin.key_fingerprint.as_deref(), Some("ab12"));
}

#[test]
fn a_mismatch_names_the_lock_id_and_never_the_manifest_text() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("evil", "1.0.0", TABLE));
    let error = load_from(&path_locked(), dir.path(), &context).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("acme"), "{text}");
    assert!(!text.contains("evil"), "{text}");
}
#[test]
fn a_path_source_plugin_has_no_manifest_digest_and_no_key() {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest("acme", "1.0.0", TABLE));
    let (plugin, _) = load_from(&path_locked(), dir.path(), &context).unwrap();
    assert_eq!(plugin.manifest_digest, None);
    assert_eq!(plugin.key_id, None);
}

/// The tree hash is the caller's: the loader never replaces it with a hash of its own, so a
/// caller that pins trust to the synced tree keeps what it pinned.
#[test]
fn the_tree_in_the_plugin_is_the_hash_the_caller_passed() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path(), &manifest("acme", "1.0.0", TABLE));
    let pinned =
        slopvac_plugins::extract::TreeHash::parse(&format!("sha256:{}", "ab".repeat(32))).unwrap();
    let context = CodePluginContext::new(pinned.clone(), PATH_ENV);
    let (plugin, _) = load_from(&path_locked(), dir.path(), &context).unwrap();
    assert_eq!(plugin.tree, pinned);
    assert_ne!(plugin.tree, tree_hash(dir.path()).unwrap());
}

#[rstest]
#[case::data_plugin("data", manifest("acme", "1.0.0", TABLE), "plugin-loader-not-code")]
#[case::id_differs("code", manifest("evil", "1.0.0", TABLE), "plugin-manifest-mismatch")]
#[case::version_differs("code", manifest("acme", "2.0.0", TABLE), "plugin-manifest-mismatch")]
#[case::unsafe_command(
    "code",
    manifest("acme", "1.0.0", &TABLE.replace("bin/acme", "../x")),
    "plugin-loader-command-unsafe"
)]
#[case::invalid_manifest(
    "code",
    "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[provides]\n".to_owned(),
    "plugin-loader-manifest-invalid"
)]
fn a_refusal_reports_its_catalog_code(
    #[case] lock_kind: &str,
    #[case] manifest_text: String,
    #[case] code: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let context = install(dir.path(), &manifest_text);
    let lock = locked(lock_kind, "source = \"plugins/acme\"\n");
    let error = load_from(&lock, dir.path(), &context).unwrap_err();
    assert_eq!(error.code(), code);
}

#[test]
fn a_tree_without_a_manifest_reports_the_unreadable_code() {
    let dir = tempfile::tempdir().unwrap();
    let context = CodePluginContext::new(tree_hash(dir.path()).unwrap(), PATH_ENV);
    let error = load_from(&path_locked(), dir.path(), &context).unwrap_err();
    assert_eq!(error.code(), "plugin-loader-manifest-unreadable");
}
