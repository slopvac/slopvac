//! Behaviour of the lock model through its public API.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::collections::BTreeMap;

use slopvac_plugins::lock::{
    ConfiguredPack, ConfiguredSource, Digest, Lockfile, OutOfRange, PackId, ResolvedPack, Source,
    SourceKind, Version, VersionReq, diff, entry_drift, outdated, parse_version_req,
};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn entry(id: &str, version: &str, source: &str, digest: &str) -> String {
    format!(
        "\n[[plugins]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"{version}\"\nslopvac_api = 1\nsource = \"{source}\"\ndigest = \"{digest}\"\n"
    )
}

fn lock_of(entries: &[String]) -> Lockfile {
    let plugins = if entries.is_empty() {
        "plugins = []\n"
    } else {
        ""
    };
    let text = format!("schema_version = 3\n{plugins}{}", entries.concat());
    Lockfile::parse(&text).unwrap()
}

fn id(s: &str) -> PackId {
    s.parse().unwrap()
}

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

fn resolved(pack: &str, version: &str, source: &str, digest: &str) -> ResolvedPack {
    ResolvedPack {
        id: id(pack),
        version: v(version),
        source: source.parse::<Source>().unwrap(),
        digest: digest.parse::<Digest>().unwrap(),
    }
}

// ---- round trip -----------------------------------------------------------

/// `full.toml` from the conformance set lists `archive` after `local`. The
/// canonical text sorts by id, writes `requires` before the signature table,
/// and separates entries with one blank line.
const FULL_CANONICAL: &str = "schema_version = 3

[[plugins]]
id = \"acme-docs\"
kind = \"data\"
version = \"1.2.0\"
slopvac_api = 1
source = \"oci://ghcr.io/acme/slopvac-docs:1.2.0\"
digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"
requires = [\"core\"]

[plugins.signature]
key_id = \"A1B2C3D4E5F60718\"
not_before = \"2026-11-01T00:00:00Z\"

[[plugins]]
id = \"acme-tagger\"
kind = \"code\"
version = \"0.3.1\"
slopvac_api = 1
source = \"git+https://github.com/acme/tagger@0123456789abcdef0123456789abcdef01234567\"
digest = \"sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210\"

[[plugins]]
id = \"archive\"
kind = \"data\"
version = \"2.0.0\"
slopvac_api = 1
source = \"https://example.org/packs/archive-2.0.0.tar.gz\"
digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"

[[plugins]]
id = \"local\"
kind = \"data\"
version = \"0.0.1\"
slopvac_api = 1
source = \"packs/local\"
digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"
";

#[test]
fn unsorted_input_is_written_in_canonical_order() {
    let full = include_str!("../../../schemas/v3/conformance/plugins-lock/valid/full.toml");
    let lock = Lockfile::parse(full).unwrap();
    assert_eq!(lock.to_toml(), FULL_CANONICAL);
}

#[test]
fn canonical_text_round_trips_byte_for_byte() {
    let lock = Lockfile::parse(FULL_CANONICAL).unwrap();
    assert_eq!(lock.to_toml(), FULL_CANONICAL);
}

#[test]
fn serialise_then_parse_gives_an_equal_lock() {
    let lock = Lockfile::parse(include_str!(
        "../../../schemas/v3/conformance/plugins-lock/valid/full.toml"
    ))
    .unwrap();
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

#[test]
fn empty_lock_round_trips() {
    let text = "schema_version = 3\nplugins = []\n";
    let lock = Lockfile::parse(text).unwrap();
    assert_eq!(lock.packs().len(), 0);
    assert_eq!(lock.to_toml(), text);
}

#[test]
fn path_with_a_quote_is_escaped_on_write() {
    // The path `packs/a"b` is valid; the writer must escape the quote.
    let text = entry("p", "1.0.0", "x", D1).replace("\"x\"", "'packs/a\"b'");
    let lock = Lockfile::parse(&format!("schema_version = 3\n{text}")).unwrap();
    assert_eq!(
        lock.get(&id("p")).unwrap().source,
        "packs/a\"b".parse::<Source>().unwrap()
    );
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

#[test]
fn x_keys_are_ignored_and_not_written() {
    let text = format!(
        "schema_version = 3\nx-note = 1\n{}x-tool = \"a\"\n[plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\nx-extra = true\n",
        entry("p", "1.0.0", "oci://ghcr.io/a/b:1", D1)
    );
    let lock = Lockfile::parse(&text).unwrap();
    assert!(!lock.to_toml().contains("x-"));
    assert!(lock.get(&id("p")).unwrap().signature.is_some());
}

#[test]
fn requires_are_sorted_in_the_model() {
    let text = format!(
        "schema_version = 3\n{}requires = [\"zeta\", \"alpha\"]\n",
        entry("p", "1.0.0", "oci://ghcr.io/a/b:1", D1)
    );
    let lock = Lockfile::parse(&text).unwrap();
    let ids: Vec<&str> = lock
        .get(&id("p"))
        .unwrap()
        .requires
        .iter()
        .map(PackId::as_str)
        .collect();
    assert_eq!(ids, ["alpha", "zeta"]);
}

// ---- rejections beyond the conformance set ---------------------------------

fn with_entry_lines(extra: &str) -> String {
    format!(
        "schema_version = 3\n{}{extra}",
        entry("p", "1.0.0", "oci://ghcr.io/a/b:1", D1)
    )
}

#[test]
fn rejects_what_the_contract_rejects() {
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "unknown root key",
            "schema_version = 3\nplugins = []\nextra = 1\n".into(),
            "",
        ),
        (
            "schema version 2",
            "schema_version = 2\nplugins = []\n".into(),
            "/schema_version",
        ),
        ("missing packs", "schema_version = 3\n".into(), ""),
        (
            "duplicate id",
            format!(
                "schema_version = 3\n{}{}",
                entry("p", "1.0.0", "oci://ghcr.io/a/b:1", D1),
                entry("p", "1.0.1", "oci://ghcr.io/a/b:2", D2)
            ),
            "/plugins/1/id",
        ),
        (
            "build metadata in version",
            with_entry_lines("").replace("1.0.0", "1.0.0+build"),
            "/plugins/0/version",
        ),
        (
            "leading zero in version",
            with_entry_lines("").replace("1.0.0", "01.0.0"),
            "/plugins/0/version",
        ),
        (
            "api zero",
            with_entry_lines("").replace("slopvac_api = 1", "slopvac_api = 0"),
            "/plugins/0/slopvac_api",
        ),
        (
            "uppercase pack id",
            with_entry_lines("").replace("id = \"p\"", "id = \"P\""),
            "/plugins/0/id",
        ),
        (
            "retired kind",
            with_entry_lines("").replace("\"data\"", "\"plugin\""),
            "/plugins/0/kind",
        ),
        (
            "repeated requires",
            with_entry_lines("requires = [\"a\", \"a\"]\n"),
            "/plugins/0/requires/1",
        ),
        (
            "29 February in a non-leap year",
            with_entry_lines(
                "[plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-02-29T00:00:00Z\"\n",
            ),
            "/plugins/0/signature/not_before",
        ),
        (
            "lowercase key id",
            with_entry_lines(
                "[plugins.signature]\nkey_id = \"a1b2c3d4e5f60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\n",
            ),
            "/plugins/0/signature/key_id",
        ),
        (
            "unknown signature key",
            with_entry_lines(
                "[plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\ntrusted = true\n",
            ),
            "/plugins/0/signature",
        ),
    ];
    for (label, text, pointer) in cases {
        let err = Lockfile::parse(&text).expect_err(label);
        assert_eq!(err.pointer, pointer, "{label}: {err}");
    }
}

#[test]
fn accepts_29_february_in_a_leap_year() {
    let text = with_entry_lines(
        "[plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2028-02-29T23:59:59Z\"\n",
    );
    assert!(Lockfile::parse(&text).is_ok());
}

#[test]
fn source_forms_are_accepted_or_rejected_as_the_schema_says() {
    let commit_url = format!("git+https://github.com/a/b@{COMMIT}");
    let upper_commit = format!("git+https://github.com/a/b@{}", COMMIT.to_uppercase());
    let long_commit = format!("git+https://github.com/a/b@{COMMIT}0");
    let accept = [
        "oci://ghcr.io/a/b:1",
        "https://example.org/p.tar.gz",
        commit_url.as_str(),
        "packs/local",
        "./packs/local",
        "packs/a..b",
    ];
    let reject = [
        "http://example.org/p.tar.gz",
        "ftp://example.org/p",
        "git+https://github.com/a/b",
        upper_commit.as_str(),
        long_commit.as_str(),
        "git+https://@0123456789abcdef0123456789abcdef01234567",
        "git+ssh://github.com/a/b",
        "oci://",
        "/abs",
        "C:/abs",
        "c:rel",
        "a\\b",
        "..",
        "a/../b",
        "a/..",
        " lead",
        "",
    ];
    for source in accept {
        assert!(source.parse::<Source>().is_ok(), "accept {source:?}");
    }
    for source in reject {
        assert!(source.parse::<Source>().is_err(), "reject {source:?}");
    }
}

#[test]
fn source_display_reproduces_the_stored_text() {
    let text = format!("git+https://github.com/a/b@{COMMIT}");
    let source: Source = text.parse().unwrap();
    assert_eq!(
        source.kind(),
        &SourceKind::Git {
            url: "https://github.com/a/b".into(),
            commit: COMMIT.into()
        }
    );
    assert_eq!(source.to_string(), text);
}

// ---- diff --------------------------------------------------------------

fn base_lock() -> Lockfile {
    lock_of(&[
        entry("alpha", "1.0.0", "oci://ghcr.io/a/alpha:1.0.0", D1),
        entry("beta", "2.0.0", "oci://ghcr.io/a/beta:2.0.0", D1),
    ])
}

fn base_resolved() -> Vec<ResolvedPack> {
    vec![
        resolved("alpha", "1.0.0", "oci://ghcr.io/a/alpha:1.0.0", D1),
        resolved("beta", "2.0.0", "oci://ghcr.io/a/beta:2.0.0", D1),
    ]
}

#[test]
fn identical_sets_have_no_diff() {
    let d = diff(&base_lock(), &base_resolved());
    assert!(d.is_empty());
    assert_eq!(d.to_string(), "");
}

#[test]
fn diff_reports_an_added_pack() {
    let mut r = base_resolved();
    r.push(resolved("gamma", "0.1.0", "packs/gamma", D2));
    let d = diff(&base_lock(), &r);
    assert_eq!(d.added, [resolved("gamma", "0.1.0", "packs/gamma", D2)]);
    assert!(d.removed.is_empty() && d.changed.is_empty());
    assert_eq!(d.to_string(), "added: gamma 0.1.0 is not in the lock\n");
}

#[test]
fn diff_reports_a_removed_pack() {
    let mut r = base_resolved();
    r.remove(0);
    let d = diff(&base_lock(), &r);
    assert_eq!(d.removed, [id("alpha")]);
    assert!(d.added.is_empty() && d.changed.is_empty());
    assert_eq!(
        d.to_string(),
        "removed: alpha is locked but was not resolved\n"
    );
}

#[test]
fn diff_reports_a_digest_change_and_only_that() {
    let mut r = base_resolved();
    r[1] = resolved("beta", "2.0.0", "oci://ghcr.io/a/beta:2.0.0", D2);
    let d = diff(&base_lock(), &r);
    assert_eq!(d.changed.len(), 1);
    let change = &d.changed[0];
    assert_eq!(change.id, id("beta"));
    assert!(change.version.is_none() && change.source.is_none());
    let digest = change.digest.as_ref().unwrap();
    assert_eq!((digest.locked.as_str(), digest.resolved.as_str()), (D1, D2));
    assert_eq!(
        d.to_string(),
        format!("changed: beta digest {D1} -> {D2}\n")
    );
}

#[test]
fn diff_reports_version_and_source_changes_together() {
    let mut r = base_resolved();
    r[0] = resolved("alpha", "1.1.0", "oci://ghcr.io/a/alpha:1.1.0", D1);
    let d = diff(&base_lock(), &r);
    assert_eq!(
        d.to_string(),
        "changed: alpha version 1.0.0 -> 1.1.0 source oci://ghcr.io/a/alpha:1.0.0 -> oci://ghcr.io/a/alpha:1.1.0\n"
    );
}

#[test]
fn diff_lists_are_sorted_by_id_whatever_the_input_order() {
    let lock = lock_of(&[]);
    let r = vec![
        resolved("zed", "1.0.0", "packs/z", D1),
        resolved("abc", "1.0.0", "packs/a", D1),
    ];
    let d = diff(&lock, &r);
    let ids: Vec<&str> = d.added.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["abc", "zed"]);
}

// ---- outdated ----------------------------------------------------------

fn req(s: &str) -> VersionReq {
    parse_version_req(s).unwrap()
}

/// One locked pack `p` at `locked`, a range, and the versions on offer.
fn run(locked: &str, range: &str, available: &[&str]) -> Vec<(Option<String>, Option<String>)> {
    let lock = lock_of(&[entry("p", locked, "oci://ghcr.io/a/p:1", D1)]);
    let ranges = BTreeMap::from([(id("p"), req(range))]);
    let avail = BTreeMap::from([(id("p"), available.iter().map(|s| v(s)).collect())]);
    outdated(&lock, &ranges, &avail)
        .into_iter()
        .map(|o| {
            (
                o.update.map(|v| v.to_string()),
                o.newest.map(|v| v.to_string()),
            )
        })
        .collect()
}

fn nothing() -> Vec<(Option<String>, Option<String>)> {
    Vec::new()
}

fn pair(update: &str, newest: &str) -> Vec<(Option<String>, Option<String>)> {
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    vec![(opt(update), opt(newest))]
}

#[test]
fn update_is_the_highest_version_inside_the_range() {
    assert_eq!(
        run("1.2.0", "^1", &["1.2.0", "1.3.0", "1.4.2", "1.4.1"]),
        pair("1.4.2", "1.4.2")
    );
}

#[test]
fn newer_major_outside_the_range_is_newest_but_not_update() {
    assert_eq!(
        run("1.2.0", "^1", &["1.2.0", "1.2.5", "2.0.0"]),
        pair("1.2.5", "2.0.0")
    );
    assert_eq!(run("1.2.0", "^1", &["2.0.0"]), pair("", "2.0.0"));
}

#[test]
fn nothing_newer_is_not_outdated() {
    assert_eq!(run("1.2.0", "^1", &["1.0.0", "1.2.0"]), nothing());
}

#[test]
fn prerelease_is_skipped_unless_the_range_names_one() {
    // 1.3.0-rc.1 sorts above 1.2.0 but ^1 does not admit it, and newest ignores prereleases.
    assert_eq!(run("1.2.0", "^1", &["1.3.0-rc.1"]), nothing());
    // A range that names a prerelease of that exact 1.3.0 admits it.
    assert_eq!(
        run("1.2.0", ">=1.3.0-rc.1, <2", &["1.3.0-rc.1"]),
        pair("1.3.0-rc.1", "")
    );
    // Naming a prerelease of 1.3.0 does not admit prereleases of 1.4.0.
    assert_eq!(run("1.2.0", ">=1.3.0-rc.1, <2", &["1.4.0-rc.1"]), nothing());
}

#[test]
fn stable_release_is_preferred_over_its_prerelease() {
    assert_eq!(
        run("1.2.0", ">=1.3.0-rc.1, <2", &["1.3.0-rc.1", "1.3.0"]),
        pair("1.3.0", "1.3.0")
    );
}

#[test]
fn packs_without_a_range_or_without_offers_are_skipped() {
    let lock = lock_of(&[
        entry("a", "1.0.0", "packs/a", D1),
        entry("b", "1.0.0", "packs/b", D1),
    ]);
    let ranges = BTreeMap::from([(id("b"), req("^1"))]);
    let avail = BTreeMap::from([(id("a"), vec![v("2.0.0")])]);
    assert_eq!(outdated(&lock, &ranges, &avail), Vec::new());
}

#[test]
fn outdated_result_is_sorted_by_id() {
    let lock = lock_of(&[
        entry("b", "1.0.0", "packs/b", D1),
        entry("a", "1.0.0", "packs/a", D1),
    ]);
    let ranges = BTreeMap::from([(id("a"), req("^1")), (id("b"), req("^1"))]);
    let avail = BTreeMap::from([(id("a"), vec![v("1.1.0")]), (id("b"), vec![v("1.1.0")])]);
    let ids: Vec<String> = outdated(&lock, &ranges, &avail)
        .iter()
        .map(|o| o.id.to_string())
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn version_requirements_follow_the_contract_grammar() {
    for ok in [
        "1",
        "1.2",
        "^1.2.3",
        "~1.2",
        "=1.2.3",
        ">=1.2, <2",
        ">= 1.2.0",
        "<=1.0.0-rc.1",
        ">1",
    ] {
        assert!(parse_version_req(ok).is_ok(), "accept {ok:?}");
    }
    for bad in [
        "", "*", "1.*", "1.x", "latest", "1.2.3.4", "^^1", "1,", ">=", "v1.2", "1.2-",
    ] {
        assert!(parse_version_req(bad).is_err(), "reject {bad:?}");
    }
}

#[test]
fn a_bare_version_is_a_caret_requirement() {
    assert_eq!(
        run("1.2.0", "1.2", &["1.9.0", "2.0.0"]),
        pair("1.9.0", "2.0.0")
    );
}

#[test]
fn a_path_source_never_has_a_colon_in_any_segment() {
    // On Windows `Path::push("C:x")` replaces or rebases the path, and `a:b` names a
    // stream, so a drive or stream form after an ordinary segment would leave the
    // project root. The check runs on every platform, so a lock reads the same everywhere.
    for source in [
        "packs/C:x",
        "packs/C:",
        "dir/a:b",
        "a/b/c:d/e",
        "packs/C:..",
        "x/:",
    ] {
        assert!(source.parse::<Source>().is_err(), "accepted {source:?}");
    }
}

#[test]
fn config_drift_keeps_a_locked_pack_that_a_configured_pack_requires() {
    let text = format!(
        "schema_version = 3\n\
         [[plugins]]\nid = \"app\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"plugins/app\"\ndigest = \"{D1}\"\nrequires = [\"base\"]\n\
         [[plugins]]\nid = \"base\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"plugins/base\"\ndigest = \"{D2}\"\n\
         [[plugins]]\nid = \"stale\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"plugins/stale\"\ndigest = \"{D1}\"\n"
    );
    let lock = Lockfile::parse(&text).unwrap();
    let configured: Vec<PackId> = ["app", "new"]
        .iter()
        .map(|id| id.parse().unwrap())
        .collect();
    let drift = slopvac_plugins::lock::config_drift(&lock, &configured);
    let ids = |v: &[PackId]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(ids(&drift.unlocked), ["new"]);
    assert_eq!(ids(&drift.removed), ["stale"]);
}

// ---- drift between the config entries and the lock --------------------------

/// The project root of the drift tests. A config file's directory is given relative to it.
const ROOT: &str = "/work/project";

fn parse_source(
    text: &str,
    base: &str,
) -> Result<ConfiguredSource, slopvac_plugins::lock::ValueError> {
    let root = std::path::Path::new(ROOT);
    ConfiguredSource::parse(text, root, &root.join(base))
}

fn configured(pack: &str, source: &str, base: &str, range: Option<&str>) -> ConfiguredPack {
    ConfiguredPack::new(
        id(pack),
        parse_source(source, base).unwrap(),
        range.map(|r| parse_version_req(r).unwrap()),
    )
}

/// The ids whose configured source names another source than the lock entry.
fn source_drift(locked_source: &str, config_source: &str, base: &str) -> Vec<String> {
    let lock = lock_of(&[entry("x", "1.2.0", locked_source, D1)]);
    let drift = entry_drift(&lock, &[configured("x", config_source, base, None)]);
    drift
        .source_differs
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// An `oci://` source compares the registry and the repository only, with any tag or digest
/// on either side dropped. A registry port is part of the registry.
#[rstest::rstest]
#[case::documented_untagged_config_and_a_tagged_lock(
    "oci://ghcr.io/acme/slopvac-acme:1.2.0",
    "oci://ghcr.io/acme/slopvac-acme"
)]
#[case::tag_against_digest(
    "oci://ghcr.io/acme/x@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "oci://ghcr.io/acme/x:2.0.0"
)]
#[case::port_kept("oci://reg.example:5000/acme/x:1.2.0", "oci://reg.example:5000/acme/x")]
#[case::host_in_another_case("oci://ghcr.io/acme/x:1.2.0", "oci://GHCR.IO/acme/x")]
#[case::host_and_port_in_another_case(
    "oci://Reg.Example:5000/acme/x:1.2.0",
    "oci://reg.example:5000/acme/x"
)]
fn an_oci_source_with_the_same_repository_matches(#[case] locked: &str, #[case] config: &str) {
    assert_eq!(source_drift(locked, config, ""), Vec::<String>::new());
}

#[rstest::rstest]
#[case::other_repository("oci://ghcr.io/acme/x:1.2.0", "oci://ghcr.io/acme/y")]
#[case::other_registry("oci://ghcr.io/acme/x:1.2.0", "oci://quay.io/acme/x")]
#[case::other_port("oci://reg.example:5000/acme/x:1.2.0", "oci://reg.example:6000/acme/x")]
#[case::port_against_none("oci://reg.example:5000/acme/x:1.2.0", "oci://reg.example/acme/x")]
#[case::git_other_commit(
    "git+https://example.com/acme/x@0123456789abcdef0123456789abcdef01234567",
    "git+https://example.com/acme/x@fedcba9876543210fedcba9876543210fedcba98"
)]
#[case::git_other_repository(
    "git+https://example.com/acme/x@0123456789abcdef0123456789abcdef01234567",
    "git+https://example.com/acme/y@0123456789abcdef0123456789abcdef01234567"
)]
#[case::archive_other_url(
    "https://example.com/x-1.2.0.tar.gz",
    "https://example.com/x-1.3.0.tar.gz"
)]
#[case::archive_with_a_query("https://example.com/x.tar.gz", "https://example.com/x.tar.gz?v=2")]
#[case::path_other_directory("plugins/x", "plugins/y")]
#[case::path_against_oci("plugins/x", "oci://ghcr.io/acme/x")]
#[case::lock_with_an_empty_tag_and_a_digest(
    "oci://ghcr.io/acme/x:@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "oci://ghcr.io/acme/x"
)]
fn a_source_that_names_another_source_is_drift(#[case] locked: &str, #[case] config: &str) {
    assert_eq!(source_drift(locked, config, ""), ["x"]);
}

#[rstest::rstest]
#[case::git("git+https://example.com/acme/x@0123456789abcdef0123456789abcdef01234567")]
#[case::archive("https://example.com/x-1.2.0.tar.gz")]
#[case::path("plugins/x")]
fn a_source_with_the_same_text_matches(#[case] source: &str) {
    assert_eq!(source_drift(source, source, ""), Vec::<String>::new());
}

/// The lock accepts a path with `.` segments and a trailing slash. It names the same directory
/// as the canonical form, so it matches the configured path.
#[rstest::rstest]
#[case::leading_dot("./plugins/x")]
#[case::trailing_slash("plugins/x/")]
#[case::inner_dot("plugins/./x")]
fn a_non_canonical_lock_path_matches_the_same_directory(#[case] locked: &str) {
    assert_eq!(source_drift(locked, "plugins/x", ""), Vec::<String>::new());
}

/// A config path is relative to the file that declares it; the lock path to the project root.
#[rstest::rstest]
#[case::root_config("plugins/x", "")]
#[case::dot_segments("./plugins/./x/", "")]
#[case::file_in_a_subdirectory("../plugins/x", "config")]
#[case::file_two_levels_down("../../plugins/x", "config/shared")]
#[case::file_outside_the_root("../project/plugins/x", "../shared")]
fn a_path_resolved_against_its_config_file_matches(#[case] config: &str, #[case] base: &str) {
    assert_eq!(
        source_drift("plugins/x", config, base),
        Vec::<String>::new()
    );
}

#[rstest::rstest]
#[case::subdirectory_path_read_from_the_root("../plugins/x", "")]
#[case::root_path_read_from_a_subdirectory("plugins/x", "config")]
#[case::outside_the_root("../x", "")]
fn a_path_that_resolves_elsewhere_is_drift(#[case] config: &str, #[case] base: &str) {
    assert_eq!(source_drift("plugins/x", config, base), ["x"]);
}

/// The forms the lock refuses are refused in the config too, and the refusal never repeats
/// the text.
#[rstest::rstest]
#[case::http("http://example.com/x.tar.gz")]
#[case::git_without_a_commit("git+https://example.com/acme/x")]
#[case::git_branch("git+https://example.com/acme/x@main")]
#[case::unknown_scheme("ftp://example.com/x")]
#[case::user_info("https://reader:hunter2-secret@example.com/x.tar.gz")]
#[case::oci_user_info("oci://reader:hunter2-secret@ghcr.io/acme/x")]
#[case::absolute_path("/srv/plugins/x")]
#[case::empty("")]
#[case::oci_host_only("oci://ghcr.io")]
#[case::oci_empty_tag("oci://ghcr.io/acme/x:")]
#[case::oci_bad_tag("oci://ghcr.io/acme/x:-1")]
#[case::oci_bad_digest("oci://ghcr.io/acme/x@sha256:short")]
#[case::oci_empty_tag_and_a_digest(
    "oci://ghcr.io/acme/x:@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)]
#[case::oci_bad_tag_and_a_digest(
    "oci://ghcr.io/acme/x:-1@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)]
#[case::oci_uppercase_repository("oci://ghcr.io/Acme/x")]
fn a_source_the_lock_would_refuse_is_invalid(#[case] text: &str) {
    let result = parse_source(text, "");
    assert!(result.is_err(), "accepted {text:?} as {result:?}");
    let shown = result.map(|_| ()).unwrap_err();
    for shown in [shown.to_string(), format!("{shown:?}")] {
        assert!(!shown.contains("hunter2-secret"), "{shown}");
    }
}

#[rstest::rstest]
#[case::caret_allows("^1.2", false)]
#[case::exact_allows("=1.2.0", false)]
#[case::next_major("^2", true)]
#[case::upper_bound_excludes(">=1.0, <1.2", true)]
#[case::higher_floor(">=1.3", true)]
fn a_locked_version_outside_the_requirement_is_drift(#[case] range: &str, #[case] drifts: bool) {
    let lock = lock_of(&[entry("x", "1.2.0", "plugins/x", D1)]);
    let drift = entry_drift(&lock, &[configured("x", "plugins/x", "", Some(range))]);
    let expected: Vec<OutOfRange> = if drifts {
        vec![OutOfRange::new(
            id("x"),
            v("1.2.0"),
            parse_version_req(range).unwrap(),
        )]
    } else {
        Vec::new()
    };
    assert_eq!(drift.out_of_range, expected);
    assert_eq!(drift.is_empty(), !drifts);
}

#[test]
fn an_entry_with_no_requirement_accepts_any_locked_version() {
    let lock = lock_of(&[entry("x", "0.0.1-alpha.1", "plugins/x", D1)]);
    let drift = entry_drift(&lock, &[configured("x", "plugins/x", "", None)]);
    assert!(drift.is_empty(), "{drift:?}");
}

/// An entry that the lock lacks is the `unlocked` case of the id check, not a source or range
/// difference.
#[test]
fn an_entry_with_no_lock_entry_has_no_source_or_range_drift() {
    let lock = lock_of(&[entry("x", "1.2.0", "plugins/x", D1)]);
    let drift = entry_drift(&lock, &[configured("y", "plugins/y", "", Some("^9"))]);
    assert!(drift.is_empty(), "{drift:?}");
}

/// The configured source can carry a credential in its query, so the difference names the id
/// and never the value: not in the message, the debug form or the error chain.
#[test]
fn source_drift_never_repeats_the_configured_source() {
    let lock = lock_of(&[entry("x", "1.2.0", "https://example.com/x.tar.gz", D1)]);
    let drift = entry_drift(
        &lock,
        &[configured(
            "x",
            "https://example.com/x.tar.gz?token=hunter2-secret",
            "",
            None,
        )],
    );
    assert_eq!(
        drift.to_string(),
        "source: x is configured with a different source from its lock entry\n"
    );
    let err = slopvac_plugins::sync::SyncError::LockMismatch(Box::new(drift));
    let mut shown = vec![err.to_string(), format!("{err:?}")];
    let mut cause = std::error::Error::source(&err);
    while let Some(e) = cause {
        shown.push(e.to_string());
        shown.push(format!("{e:?}"));
        cause = e.source();
    }
    for text in shown {
        assert!(!text.contains("hunter2-secret"), "{text}");
    }
}

/// The debug form of a configured source names its kind and, for a URL or an OCI reference,
/// the host and port. It leaves out the path, the repository, the query and the fragment, any
/// of which can carry a credential.
#[rstest::rstest]
#[case::archive(
    "https://example.com:8443/hunter2-secret/x.tar.gz?token=hunter2-secret#frag-hunter2-secret",
    &["archive", "example.com:8443"]
)]
#[case::archive_with_an_at_sign_in_the_query(
    "https://example.com/x.tar.gz?u=reader@hunter2-secret",
    &["archive", "example.com"]
)]
#[case::git(
    "git+https://example.com/hunter2-secret/x@0123456789abcdef0123456789abcdef01234567",
    &["git", "example.com"]
)]
#[case::oci("oci://reg.example:5000/hunter2-secret/x:1.0.0", &["oci", "reg.example:5000"])]
#[case::path("plugins/hunter2-secret", &["path"])]
fn the_debug_form_of_a_configured_source_names_only_kind_and_host(
    #[case] text: &str,
    #[case] kept: &[&str],
) {
    let pack = configured("x", text, "", None);
    for shown in [format!("{:?}", pack.source), format!("{pack:?}")] {
        assert!(!shown.contains("hunter2-secret"), "{shown}");
        assert!(!shown.contains("x.tar.gz"), "{shown}");
        for part in kept {
            assert!(shown.contains(part), "{part} missing from {shown}");
        }
    }
}
