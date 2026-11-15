//! Plugin-origin text in error messages (threat model O1, P15, TM-11; bead 9.29's sites).
//!
//! Every error type that interpolates text from a plugin, a registry, a cache
//! or a lock file must show it escaped. Each case below puts the same hostile
//! string into string fields and checks the whole message.

use slopvac_plugins::apply::ApplyError;
use slopvac_plugins::extract::ExtractError;
use slopvac_plugins::frame::FrameError;
use slopvac_plugins::oci::OciError;
use slopvac_plugins::sign::SignError;
use slopvac_plugins::sync::SyncError;
use slopvac_plugins::trust::TrustError;
use slopvac_plugins::{PluginError, RpcError};

/// ESC, an OSC 8 opener with BEL, a C1 CSI, a bidi override, a zero-width space,
/// a newline that would start a workflow command, and a legacy command.
const HOSTILE: &str =
    "\u{1b}]8;;https://e.example\u{7}\u{9b}\u{202e}\u{200b}\n::error::x! ##[add-mask]z";
/// What the one-line escaper must make of `HOSTILE`, worked out by hand.
const ESCAPED: &str = "\\u{1b}]8;;https://e.example\\u{7}\\u{9b}\\u{202e}\\u{200b}\\u{a}::error::x! \\u{23}#[add-mask]z";

fn cases() -> Vec<(&'static str, String)> {
    let h = || HOSTILE.to_owned();
    let io = || std::io::Error::other("io");
    vec![
        ("Untrusted", PluginError::Untrusted(h()).to_string()),
        ("HashMismatch", PluginError::HashMismatch(h()).to_string()),
        (
            "Timeout",
            PluginError::Timeout {
                plugin: "p".into(),
                what: h(),
            }
            .to_string(),
        ),
        (
            "ProtocolViolation",
            PluginError::ProtocolViolation(h()).to_string(),
        ),
        ("OutputInvalid", PluginError::OutputInvalid(h()).to_string()),
        ("SpawnFailed", PluginError::SpawnFailed(h()).to_string()),
        (
            "Rpc",
            PluginError::Rpc(RpcError {
                code: -1,
                message: h(),
                slopvac_code: "c".into(),
                data: serde_json::Value::Null,
            })
            .to_string(),
        ),
        ("Usage", PluginError::Usage(h()).to_string()),
        ("Framing", FrameError::Framing(h()).to_string()),
        ("Extract::Refused", ExtractError::Refused(h()).to_string()),
        (
            "Extract::Malformed",
            ExtractError::Malformed(h()).to_string(),
        ),
        (
            "Extract::Io",
            ExtractError::Io {
                what: h(),
                source: io(),
            }
            .to_string(),
        ),
        ("Sign::Mismatch", SignError::Mismatch(h()).to_string()),
        ("Sign::Rollback", SignError::Rollback(h()).to_string()),
        (
            "Sign::KeyNotPinned",
            SignError::KeyNotPinned {
                key_id: h(),
                namespace: h(),
            }
            .to_string(),
        ),
        ("Trust::Program", TrustError::Program(h()).to_string()),
        ("Trust::NotAHash", TrustError::NotAHash(h()).to_string()),
        ("Oci::Invalid", OciError::Invalid(h()).to_string()),
        ("Oci::NotFound", OciError::NotFound(h()).to_string()),
        (
            "Oci::Network",
            OciError::Network {
                url: h(),
                message: h(),
            }
            .to_string(),
        ),
        ("Sync::Store", SyncError::Store(h()).to_string()),
        ("Apply::Io", ApplyError::Io(h()).to_string()),
        ("Apply::Symlink", ApplyError::Symlink(h()).to_string()),
        (
            "Apply::CaseCollision",
            ApplyError::CaseCollision {
                path: h(),
                existing: h(),
            }
            .to_string(),
        ),
    ]
}

#[test]
fn hostile_text_in_any_error_message_is_escaped() {
    for (name, msg) in cases() {
        let bad = msg.chars().find(|&c| slopvac_sanitize::is_unsafe(c));
        assert_eq!(bad, None, "{name}: {msg:?}");
        assert!(msg.contains(ESCAPED), "{name}: {msg:?}");
    }
}

#[test]
fn an_exited_plugin_keeps_its_stderr_lines_and_escapes_the_rest() {
    let msg = PluginError::Exited {
        plugin: "p".into(),
        status: "code 3".into(),
        stderr: format!("\nstderr:\n{HOSTILE}\nline two"),
    }
    .to_string();
    let want = "plugin p exited (code 3)\nstderr:\n\\u{1b}]8;;https://e.example\\u{7}\\u{9b}\\u{202e}\\u{200b}\n\\u{3a}:error::x! \\u{23}#[add-mask]z\nline two";
    assert_eq!(msg, want);
}

#[test]
#[cfg(unix)]
fn a_link_name_the_name_check_lets_through_is_escaped_in_the_extract_error() {
    // A soft hyphen and a byte order mark pass the name check, so the
    // link rejection interpolates the name itself.
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("elsewhere", dir.path().join("a\u{ad}b\u{feff}")).unwrap();
    let err = slopvac_plugins::extract::tree_hash(dir.path()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "archive refused: `a\\u{ad}b\\u{feff}` is a link or special file"
    );
}

#[test]
fn a_literal_backslash_in_an_error_field_cannot_pose_as_an_escape() {
    let msg = PluginError::Untrusted("\\u{202e}".into()).to_string();
    assert!(
        msg.starts_with("plugin \\\\u{202e} is not trusted"),
        "{msg}"
    );
}

#[test]
fn declaration_differences_escape_manifest_names() {
    use slopvac_plugins::trust::DeclSummary;
    let summary = |hooks: &[&str]| DeclSummary {
        network: false,
        hooks: hooks.iter().map(|h| (*h).to_owned()).collect(),
        required_features: vec![],
        optional_features: vec![],
    };
    let changes = summary(&[]).diff(&summary(&["tag\u{202e}\n::error::x"]));
    assert_eq!(changes, ["hook added: tag\\u{202e}\\u{a}::error::x"]);
}

// ---- composed and nested paths (review round 2) ----

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_recipe_path_in_the_approval_list_is_escaped_not_debug_printed() {
    use slopvac_plugins::apply::{EntryStatus, PlanOptions, apply_plan};
    use slopvac_plugins::hooks::parse_recipe;
    // A workflow path is on the deny list; `##[` is a legacy runner command.
    let result = serde_json::json!({"files": [{
        "path": ".github/workflows/a##[error]b.yml",
        "action": "write",
        "content": "x",
    }]});
    let files = parse_recipe(&result).unwrap();
    let root = tempfile::tempdir().unwrap();
    let options = PlanOptions {
        dry_run: true,
        allow_sensitive: vec![],
    };
    let report = apply_plan(root.path(), &files, &options);
    let EntryStatus::Refused(err) = &report.entries[0].status else {
        panic!("{:?}", report.entries[0].status);
    };
    assert_eq!(
        err.to_string(),
        "`.github/workflows/a\\u{23}#[error]b.yml` is a sensitive path; \
         approve `.github/workflows/a\\u{23}#[error]b.yml` to write it"
    );
}

#[test]
fn a_locked_source_in_the_lock_mismatch_is_escaped() {
    use slopvac_plugins::lock::{LockedPack, Lockfile, PackKind, ResolvedPack};
    use slopvac_plugins::sync::check_lock;
    let pack = |source: &str, digest: &str| LockedPack {
        id: "p".parse().unwrap(),
        kind: PackKind::Data,
        version: "1.0.0".parse().unwrap(),
        slopvac_api: 1,
        source: source.parse().unwrap(),
        digest: digest.parse().unwrap(),
        signature: None,
        requires: Vec::new(),
    };
    let locked = pack("https://example.org/a##[error]x.tgz", D1);
    let lock = Lockfile::new(vec![locked]).unwrap();
    let resolved = ResolvedPack {
        id: "p".parse().unwrap(),
        version: "1.0.0".parse().unwrap(),
        source: "https://example.org/b##[add-mask]y.tgz".parse().unwrap(),
        digest: D1.parse().unwrap(),
    };
    let msg = check_lock(&lock, &[resolved]).unwrap_err().to_string();
    assert_eq!(
        msg,
        "the lock does not match the resolved packs:\n\
         changed: p source https://example.org/a\\u{23}#[error]x.tgz \
         -> https://example.org/b\\u{23}#[add-mask]y.tgz\n"
    );
}

#[test]
#[cfg(unix)]
fn a_store_path_with_hostile_characters_is_escaped_once() {
    use slopvac_plugins::sync::Store;
    let tmp = tempfile::tempdir().unwrap();
    // U+202E, a literal backslash and a legacy command in a directory name.
    let root = tmp.path().join("s\u{202e}\\n##[e]");
    std::fs::create_dir_all(root.join(".lock")).unwrap();
    let msg = Store::new(&root).lock().err().unwrap().to_string();
    let want = "/s\\u{202e}\\\\n\\u{23}#[e]/.lock is not a plain file";
    assert!(msg.starts_with("data directory: "), "{msg}");
    assert!(msg.ends_with(want), "{msg}");
}

#[test]
#[cfg(unix)]
fn a_planted_link_in_a_store_with_a_hostile_path_is_escaped_once() {
    use slopvac_plugins::sync::Store;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("s\u{202e}\\n##[e]");
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(tmp.path(), root.join("trees")).unwrap();
    let msg = Store::new(&root).lock().err().unwrap().to_string();
    assert!(
        msg.starts_with("data directory: archive refused: "),
        "{msg}"
    );
    assert!(msg.contains("/s\\u{202e}\\\\n\\u{23}#[e]/trees"), "{msg}");
}

#[test]
fn a_digest_in_a_manifest_is_escaped_once_in_the_parse_error() {
    use slopvac_plugins::oci::{Digest, Fetched};
    let body = r#"{"schemaVersion":2,"config":{"mediaType":"x","digest":"bad\u202e\\##[","size":1},"layers":[]}"#;
    let fetched = Fetched {
        digest: Digest::of(body.as_bytes()),
        media_type: "application/vnd.oci.image.manifest.v1+json".into(),
        body: body.as_bytes().to_vec(),
    };
    let msg = fetched.manifest().unwrap_err().to_string();
    assert!(
        msg.starts_with(
            "invalid OCI input: manifest: digest `bad\\u{202e}\\\\\\u{23}#[` is not sha256"
        ),
        "{msg}"
    );
}

#[test]
fn revocation_errors_and_notices_escape_their_fields() {
    use slopvac_plugins::revoke::{RevocationError, RevocationNotice, Revoked};
    let h = || HOSTILE.to_owned();
    for (name, msg) in [
        (
            "Expired",
            RevocationError::Expired { expires: h() }.to_string(),
        ),
        (
            "Rollback",
            RevocationError::Rollback {
                cached: h(),
                fetched: h(),
            }
            .to_string(),
        ),
        (
            "Equivocation",
            RevocationError::Equivocation { issued: h() }.to_string(),
        ),
        ("Invalid", RevocationError::Invalid(h()).to_string()),
        ("Layout", RevocationError::Layout(h()).to_string()),
        ("Clock", RevocationError::Clock(h()).to_string()),
        (
            "CacheInvalid",
            RevocationError::CacheInvalid {
                path: h().into(),
                why: h(),
            }
            .to_string(),
        ),
        ("Revoked::Key", Revoked::Key(h()).to_string()),
        ("Revoked::Digest", Revoked::Digest(h()).to_string()),
        (
            "Stale",
            RevocationNotice::Stale {
                issued: h(),
                expires: h(),
            }
            .to_string(),
        ),
    ] {
        assert_eq!(
            msg.chars().find(|&c| slopvac_sanitize::is_unsafe(c)),
            None,
            "{name}: {msg:?}"
        );
        assert!(msg.contains(ESCAPED), "{name}: {msg:?}");
    }
}
