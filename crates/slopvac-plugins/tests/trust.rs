//! Trust store and gate tests (H20, TM-6, TM-9, TM-22, TM-26, TM-28, TM-29, N8).
//!
//! Store, hash, revocation and gate-decision tests run on every platform; they
//! call `SpawnGate::check` and never start a process. Tests that start the fake
//! plugin or the Python SDK example through the host, or depend on Unix
//! permissions and symlinks, live in `unix_only`.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use common::decl;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use slopvac_plugins::extract::{Limits, TreeHash, evict, extract_targz};
use slopvac_plugins::oci::Digest;
use slopvac_plugins::protocol::{FeatureSet, HookDecl};
use slopvac_plugins::trust::{
    DeclSummary, Revocation, TrustError, TrustGate, TrustInputs, TrustRecord, TrustState,
    TrustStore, VerifiedPlugin, is_forbidden_env_name, parse_trust_env, trust_hash,
};
use slopvac_plugins::{PluginDeclaration, PluginError, PluginSpec, SpawnGate};

const PLUGIN_PY: &str = include_str!("fixtures/fake_plugin.py");
const NOW: &str = "2026-11-14T12:00:00Z";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("slopvac-trust-{name}-{}", std::process::id()));
        let _ = evict(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn dir(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = evict(&self.0);
    }
}

fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, name, *data).unwrap();
    }
    let bytes = tar.into_inner().unwrap();
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut enc, &bytes).unwrap();
    enc.finish().unwrap()
}

fn install(s: &Scratch, version: &str, extra: &str) -> VerifiedPlugin {
    let archive = targz(&[
        ("plugin.py", PLUGIN_PY.as_bytes()),
        ("VERSION", version.as_bytes()),
        ("extra", extra.as_bytes()),
    ]);
    let out = extract_targz(
        archive.as_slice(),
        &s.dir("cache"),
        &HashSet::new(),
        Limits::default(),
    )
    .unwrap();
    VerifiedPlugin {
        name: "fake".into(),
        version: Some(version.into()),
        source: "oci:ghcr.io/acme/fake".into(),
        root: out.root,
        tree: out.hash,
        manifest_digest: Some(Digest::of(version.as_bytes()).to_string()),
        key_fingerprint: Some("f".repeat(64)),
    }
}

/// A spec whose program is `plugin.py` inside the verified tree. Never started.
fn spec_for(vp: &VerifiedPlugin, declaration: PluginDeclaration) -> PluginSpec {
    PluginSpec {
        name: vp.name.clone(),
        program: vp.root.join("plugin.py"),
        args: vec!["ok".into(), "{}".into()],
        cwd: vp.root.clone(),
        declaration,
        path_env: std::env::join_paths([std::env::temp_dir()])
            .unwrap()
            .into_string()
            .unwrap(),
        env: vec![],
    }
}

fn trust(gate: &TrustGate, vp: &VerifiedPlugin, spec: &PluginSpec) {
    gate.record_trust(vp, spec, NOW).unwrap();
}

fn gate(s: &Scratch, vp: &VerifiedPlugin) -> TrustGate {
    TrustGate::new(TrustStore::open(s.dir("state")), vec![], vec![vp.clone()])
}

fn hash_of(g: &TrustGate, vp: &VerifiedPlugin, spec: &PluginSpec) -> Digest {
    g.state(vp, spec).unwrap().0
}

fn record(hash: &str, name: &str) -> TrustRecord {
    TrustRecord {
        hash: hash.into(),
        name: name.into(),
        version: None,
        source: "test".into(),
        key_fingerprint: None,
        declaration: DeclSummary::of(&decl()),
        recorded_at: NOW.into(),
    }
}

#[test]
fn an_untrusted_plugin_is_refused_and_names_the_command_and_hash() {
    let s = Scratch::new("untrusted");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let err = gate(&s, &vp).check(&spec).unwrap_err();
    assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("slopvac plugins trust"), "{text}");
    assert!(
        text.contains("sha256:"),
        "the message names the hash: {text}"
    );
}

#[test]
fn n8_a_failed_check_records_nothing() {
    let s = Scratch::new("n8");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    assert!(gate(&s, &vp).check(&spec).is_err());
    assert!(
        !s.dir("state").exists(),
        "checking must not create or write the store"
    );
}

#[test]
fn a_trusted_plugin_passes_the_gate() {
    let s = Scratch::new("trusted");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let g = gate(&s, &vp);
    trust(&g, &vp, &spec);
    g.check(&spec).unwrap();
}

#[test]
fn tm9_an_update_with_a_changed_declaration_needs_new_trust_and_shows_the_diff() {
    let s = Scratch::new("tm9");
    let v1 = install(&s, "1.0.0", "");
    let spec1 = spec_for(&v1, decl());
    trust(&gate(&s, &v1), &v1, &spec1);

    let v2 = install(&s, "2.0.0", "");
    let mut wider = decl();
    wider.network = true;
    wider.hooks.push(HookDecl {
        kind: "scorer".into(),
        id: "main".into(),
    });
    let spec2 = spec_for(&v2, wider);
    let g2 = gate(&s, &v2);
    let err = g2.check(&spec2).unwrap_err();
    assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
    let (_, state) = g2.state(&v2, &spec2).unwrap();
    let TrustState::Untrusted {
        changes,
        previously_trusted,
    } = state
    else {
        panic!("{state:?}")
    };
    assert!(previously_trusted);
    assert_eq!(
        changes,
        ["network: false -> true", "hook added: scorer/main"]
    );
    let records = TrustStore::open(s.dir("state")).records().unwrap();
    assert!(
        records
            .iter()
            .all(|r| r.version.as_deref() == Some("1.0.0")),
        "an update never records trust"
    );
}

#[test]
fn tm9_an_update_shows_removed_hooks_and_feature_differences_exactly() {
    let s = Scratch::new("tm9-diff");
    let features = |required: &[&str], optional: &[&str]| FeatureSet {
        required: required.iter().map(|x| (*x).to_owned()).collect(),
        optional: optional.iter().map(|x| (*x).to_owned()).collect(),
    };
    let mut old = decl();
    old.features = features(&["r1"], &["o1", "o3"]);
    let v1 = install(&s, "1.0.0", "");
    trust(&gate(&s, &v1), &v1, &spec_for(&v1, old));

    let mut new = decl();
    new.hooks.retain(|h| h.kind != "fixer");
    new.features = features(&["r2"], &["o1", "o2"]);
    let v2 = install(&s, "2.0.0", "");
    let (_, state) = gate(&s, &v2).state(&v2, &spec_for(&v2, new)).unwrap();
    let TrustState::Untrusted { changes, .. } = state else {
        panic!("{state:?}")
    };
    assert_eq!(
        changes,
        [
            "hook removed: fixer/main",
            "required feature added: r2",
            "required feature removed: r1",
            "optional feature added: o2",
            "optional feature removed: o3",
        ]
    );
}

#[test]
fn tm28_a_trust_file_in_the_project_grants_nothing() {
    let s = Scratch::new("tm28");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let rogue = TrustGate::new(
        TrustStore::open(s.dir("project/.slopvac")),
        vec![],
        vec![vp.clone()],
    );
    trust(&rogue, &vp, &spec);
    assert!(s.dir("project/.slopvac/trust.json").exists());
    let err = gate(&s, &vp).check(&spec).unwrap_err();
    assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
}

#[test]
fn tm29_slopvac_trust_accepts_content_hashes_only() {
    assert!(
        matches!(parse_trust_env("my-plugin"), Err(TrustError::NotAHash(t)) if t == "my-plugin")
    );
    assert!(matches!(
        parse_trust_env("1.0.0, latest"),
        Err(TrustError::NotAHash(_))
    ));
    assert_eq!(parse_trust_env("").unwrap(), Vec::<Digest>::new());
}

#[test]
fn tm29_a_wrong_env_hash_grants_nothing_and_the_right_one_does_without_writing() {
    let s = Scratch::new("tm29");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let wrong = Digest::of(b"some other plugin");
    let g = TrustGate::new(
        TrustStore::open(s.dir("state")),
        vec![wrong],
        vec![vp.clone()],
    );
    assert!(matches!(g.check(&spec), Err(PluginError::Untrusted(_))));

    let hash = hash_of(&gate(&s, &vp), &vp, &spec);
    let env = parse_trust_env(&format!("{}, {}", Digest::of(b"x"), hash)).unwrap();
    let g = TrustGate::new(TrustStore::open(s.dir("state")), env, vec![vp.clone()]);
    assert_eq!(g.state(&vp, &spec).unwrap().1, TrustState::TrustedByEnv);
    g.check(&spec).unwrap();
    assert!(
        !s.dir("state").exists(),
        "SLOPVAC_TRUST is read-only configuration"
    );
}

#[test]
fn a_revoked_trust_hash_is_inert_even_with_a_record() {
    let s = Scratch::new("revoked-hash");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    trust(&gate(&s, &vp), &vp, &spec);
    let hash = hash_of(&gate(&s, &vp), &vp, &spec);
    let g = gate(&s, &vp).with_revoked([Revocation::TrustHash(hash)]);
    assert_eq!(g.state(&vp, &spec).unwrap().1, TrustState::Revoked);
    assert!(matches!(g.check(&spec), Err(PluginError::Untrusted(_))));
}

fn revocations(vp: &VerifiedPlugin) -> [(&'static str, Revocation); 2] {
    [
        (
            "manifest digest",
            Revocation::ManifestDigest(
                Digest::parse(vp.manifest_digest.as_ref().unwrap()).unwrap(),
            ),
        ),
        (
            "publisher key",
            Revocation::PublisherKey(vp.key_fingerprint.clone().unwrap()),
        ),
    ]
}

#[test]
fn tm26_a_revoked_manifest_digest_or_publisher_key_stops_a_recorded_plugin() {
    let s = Scratch::new("revoked-typed");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    trust(&gate(&s, &vp), &vp, &spec);
    for (what, rev) in revocations(&vp) {
        let g = gate(&s, &vp).with_revoked([rev]);
        assert_eq!(
            g.state(&vp, &spec).unwrap().1,
            TrustState::Revoked,
            "{what}"
        );
        assert!(
            matches!(g.check(&spec), Err(PluginError::Untrusted(_))),
            "{what}"
        );
    }
}

#[test]
fn tm26_a_revocation_beats_slopvac_trust_too() {
    let s = Scratch::new("revoked-env");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let hash = hash_of(&gate(&s, &vp), &vp, &spec);
    for (what, rev) in revocations(&vp) {
        let g = TrustGate::new(
            TrustStore::open(s.dir("state")),
            vec![hash.clone()],
            vec![vp.clone()],
        )
        .with_revoked([rev]);
        assert_eq!(
            g.state(&vp, &spec).unwrap().1,
            TrustState::Revoked,
            "{what}"
        );
        assert!(g.check(&spec).is_err(), "{what}");
    }
}

#[test]
fn tm26_a_revocation_of_something_else_changes_nothing() {
    let s = Scratch::new("revoked-other");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let g = gate(&s, &vp).with_revoked([
        Revocation::ManifestDigest(Digest::of(b"another manifest")),
        Revocation::PublisherKey("0".repeat(64)),
        Revocation::TrustHash(Digest::of(b"another plugin")),
    ]);
    trust(&g, &vp, &spec);
    g.check(&spec).unwrap();
}

#[test]
fn a_plugin_that_runs_from_somewhere_else_is_refused() {
    let s = Scratch::new("cwd");
    let vp = install(&s, "1.0.0", "");
    let mut spec = spec_for(&vp, decl());
    let g = gate(&s, &vp);
    trust(&g, &vp, &spec);
    spec.cwd = s.dir("elsewhere");
    assert!(matches!(g.check(&spec), Err(PluginError::HashMismatch(_))));
}

#[test]
fn changed_arguments_or_program_need_new_trust() {
    let s = Scratch::new("args");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let g = gate(&s, &vp);
    trust(&g, &vp, &spec);
    let mut other_args = spec.clone();
    other_args.args.push("--extra".into());
    assert!(matches!(
        g.check(&other_args),
        Err(PluginError::Untrusted(_))
    ));
    let mut other_program = spec;
    other_program.program = vp.root.join("extra");
    assert!(matches!(
        g.check(&other_program),
        Err(PluginError::Untrusted(_))
    ));
}

#[test]
fn tm6_a_bare_program_name_is_refused_even_when_the_file_is_in_the_tree() {
    let s = Scratch::new("bare");
    let vp = install(&s, "1.0.0", "");
    let g = gate(&s, &vp);
    for program in ["plugin.py", "./plugin.py", "sub/../plugin.py"] {
        let mut spec = spec_for(&vp, decl());
        spec.program = PathBuf::from(program);
        let err = g.check(&spec).unwrap_err();
        assert!(matches!(err, PluginError::Usage(_)), "{program}: {err:?}");
        assert!(g.state(&vp, &spec).is_err(), "{program}");
        assert!(g.record_trust(&vp, &spec, NOW).is_err(), "{program}");
    }
}

#[test]
fn tm6_a_parent_component_in_an_absolute_program_is_refused() {
    let s = Scratch::new("dotdot");
    let vp = install(&s, "1.0.0", "");
    let mut spec = spec_for(&vp, decl());
    spec.program = vp
        .root
        .join("..")
        .join(vp.root.file_name().unwrap())
        .join("plugin.py");
    assert!(matches!(
        gate(&s, &vp).state(&vp, &spec),
        Err(TrustError::Program(_))
    ));
}

#[test]
fn tm6_a_program_that_is_not_a_file_of_the_tree_is_refused() {
    let s = Scratch::new("missing");
    let vp = install(&s, "1.0.0", "");
    let g = gate(&s, &vp);
    for program in [vp.root.join("absent"), vp.root.clone()] {
        let mut spec = spec_for(&vp, decl());
        spec.program = program.clone();
        assert!(
            matches!(g.state(&vp, &spec), Err(TrustError::Program(_))),
            "{}",
            program.display()
        );
    }
}

#[test]
fn tm6_a_program_outside_the_tree_needs_an_explicit_allowlist_entry() {
    let s = Scratch::new("external");
    let vp = install(&s, "1.0.0", "");
    let outside = s.dir("interp");
    let mut spec = spec_for(&vp, decl());
    spec.program = outside.clone();
    let plain = gate(&s, &vp);
    assert!(matches!(
        plain.state(&vp, &spec),
        Err(TrustError::Program(_))
    ));
    assert!(matches!(plain.check(&spec), Err(PluginError::Usage(_))));

    let allowed = gate(&s, &vp).with_external_programs([outside]);
    trust(&allowed, &vp, &spec);
    allowed.check(&spec).unwrap();
    assert!(
        plain.check(&spec).is_err(),
        "a record does not extend the allowlist"
    );
}

#[test]
fn tm6_the_hash_carries_a_kind_tag_and_the_tree_relative_path() {
    let s = Scratch::new("kinds");
    let vp = install(&s, "1.0.0", "");
    let inside = spec_for(&vp, decl());
    let mut outside = inside.clone();
    outside.program = s.dir("interp");
    let g = gate(&s, &vp).with_external_programs([outside.program.clone()]);
    let expect = |program: &str, spec: &PluginSpec| {
        trust_hash(&TrustInputs {
            name: &vp.name,
            tree: &vp.tree,
            declaration: &spec.declaration,
            program,
            args: &spec.args,
            env: &spec.env,
            path_env: &spec.path_env,
            manifest_digest: vp.manifest_digest.as_deref(),
            key_fingerprint: vp.key_fingerprint.as_deref(),
            interpreter: None,
            environment: None,
        })
    };
    assert_eq!(hash_of(&g, &vp, &inside), expect("tree:plugin.py", &inside));
    let abs = format!("external:{}", outside.program.display());
    assert_eq!(hash_of(&g, &vp, &outside), expect(&abs, &outside));
}

#[test]
fn tm6_loader_variables_are_refused_outright() {
    let s = Scratch::new("loader");
    let vp = install(&s, "1.0.0", "");
    let g = gate(&s, &vp);
    for name in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "PYTHONPATH",
        "pythonstartup",
        "NODE_OPTIONS",
        "RUBYOPT",
        "PERL5OPT",
        "BASH_ENV",
        "PATH",
        "Path",
    ] {
        assert!(is_forbidden_env_name(name), "{name}");
        let mut spec = spec_for(&vp, decl());
        spec.env = vec![(name.into(), "/tmp/evil".into())];
        assert!(
            matches!(g.state(&vp, &spec), Err(TrustError::Environment(_))),
            "{name}"
        );
        assert!(g.record_trust(&vp, &spec, NOW).is_err(), "{name}");
        assert!(
            matches!(g.check(&spec), Err(PluginError::Usage(_))),
            "{name}"
        );
    }
    assert!(!is_forbidden_env_name("ALLOWED_VAR"));
}

#[test]
fn tm6_an_empty_or_malformed_variable_name_is_refused() {
    let s = Scratch::new("badname");
    let vp = install(&s, "1.0.0", "");
    let g = gate(&s, &vp);
    for name in ["", "A=B", "=", "A\0B"] {
        let mut spec = spec_for(&vp, decl());
        spec.env = vec![(name.into(), "v".into())];
        assert!(
            matches!(g.state(&vp, &spec), Err(TrustError::Environment(_))),
            "{name:?}"
        );
        assert!(
            matches!(
                g.record_trust(&vp, &spec, NOW),
                Err(TrustError::Environment(_))
            ),
            "{name:?}"
        );
        assert!(
            matches!(g.check(&spec), Err(PluginError::Usage(_))),
            "{name:?}"
        );
    }
}

#[test]
fn tm6_a_path_with_a_relative_or_empty_entry_is_refused() {
    let s = Scratch::new("path");
    let vp = install(&s, "1.0.0", "");
    let g = gate(&s, &vp);
    let abs = std::env::temp_dir();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let abs = abs.display();
    for bad in [
        String::new(),
        ".".into(),
        format!("{abs}{sep}bin"),
        format!("{abs}{sep}"),
    ] {
        let mut spec = spec_for(&vp, decl());
        spec.path_env = bad.clone();
        assert!(
            matches!(g.state(&vp, &spec), Err(TrustError::Environment(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn tm6_a_duplicate_variable_name_is_refused() {
    let s = Scratch::new("dupe");
    let vp = install(&s, "1.0.0", "");
    let mut spec = spec_for(&vp, decl());
    spec.env = vec![("A".into(), "1".into()), ("a".into(), "2".into())];
    assert!(matches!(
        gate(&s, &vp).state(&vp, &spec),
        Err(TrustError::Environment(_))
    ));
}

#[test]
fn tm6_changing_a_variable_value_or_the_path_needs_new_trust() {
    let s = Scratch::new("env-hash");
    let vp = install(&s, "1.0.0", "");
    let mut spec = spec_for(&vp, decl());
    spec.env = vec![("A".into(), "1".into()), ("B".into(), "2".into())];
    let g = gate(&s, &vp);
    trust(&g, &vp, &spec);
    g.check(&spec).unwrap();

    let mut reordered = spec.clone();
    reordered.env.reverse();
    g.check(&reordered).unwrap();

    let mut value = spec.clone();
    value.env[0].1 = "other".into();
    let mut added = spec.clone();
    added.env.push(("C".into(), "3".into()));
    let mut removed = spec.clone();
    removed.env.pop();
    let mut path = spec.clone();
    path.path_env = s.dir("other-bin").display().to_string();
    for (what, changed) in [
        ("value", value),
        ("added", added),
        ("removed", removed),
        ("path", path),
    ] {
        assert!(
            matches!(g.check(&changed), Err(PluginError::Untrusted(_))),
            "{what}"
        );
    }
}

#[test]
fn every_trust_input_changes_the_hash() {
    let tree1 = TreeHash::parse(&Digest::of(b"1").to_string()).unwrap();
    let tree2 = TreeHash::parse(&Digest::of(b"2").to_string()).unwrap();
    let d = decl();
    let mut net = decl();
    net.network = true;
    let args: Vec<String> = vec!["a".into()];
    let none_args: Vec<String> = vec![];
    let env = vec![("A".to_owned(), "1".to_owned())];
    let other_env = vec![("A".to_owned(), "2".to_owned())];
    let base = TrustInputs {
        name: "p",
        tree: &tree1,
        declaration: &d,
        program: "tree:run",
        args: &args,
        env: &env,
        path_env: "/usr/bin",
        manifest_digest: Some("sha256:aa"),
        key_fingerprint: Some("fp"),
        interpreter: Some("py3.12"),
        environment: Some("env1"),
    };
    let h = trust_hash(&base);
    assert_eq!(h, trust_hash(&base.clone()), "deterministic");
    let variants = [
        TrustInputs {
            name: "q",
            ..base.clone()
        },
        TrustInputs {
            tree: &tree2,
            ..base.clone()
        },
        TrustInputs {
            declaration: &net,
            ..base.clone()
        },
        TrustInputs {
            program: "tree:other",
            ..base.clone()
        },
        TrustInputs {
            program: "external:run",
            ..base.clone()
        },
        TrustInputs {
            args: &none_args,
            ..base.clone()
        },
        TrustInputs {
            env: &other_env,
            ..base.clone()
        },
        TrustInputs {
            env: &[],
            ..base.clone()
        },
        TrustInputs {
            path_env: "/bin",
            ..base.clone()
        },
        TrustInputs {
            manifest_digest: Some("sha256:bb"),
            ..base.clone()
        },
        TrustInputs {
            manifest_digest: None,
            ..base.clone()
        },
        TrustInputs {
            key_fingerprint: Some("fp2"),
            ..base.clone()
        },
        TrustInputs {
            interpreter: Some("py3.13"),
            ..base.clone()
        },
        TrustInputs {
            environment: Some("env2"),
            ..base.clone()
        },
    ];
    let hashes: HashSet<String> = variants.iter().map(|v| trust_hash(v).to_string()).collect();
    assert_eq!(hashes.len(), variants.len());
    assert!(!hashes.contains(h.as_str()));
}

#[test]
fn the_trust_hash_matches_an_independently_derived_vector() {
    // CI users pin these hashes in SLOPVAC_TRUST, so the format is an interface.
    // The canonical input below was written by hand (sorted keys, no spaces) and
    // hashed with Python: hashlib.sha256(canonical.encode()).hexdigest().
    //
    // {"args":["a","b"],"declaration":{"features":{"optional":[],"required":[]},
    // "hooks":[{"id":"main","kind":"tagger"}],"network":false},
    // "env":[["A","1"],["B","2"]],"environment":null,"interpreter":null,
    // "key":"fp","manifest":"sha256:<64 x a>","name":"p","path":"/usr/bin:/bin",
    // "program":"tree:bin/run","tree":"sha256:<64 x 1>","v":1}
    let tree = TreeHash::parse(&format!("sha256:{}", "11".repeat(32))).unwrap();
    let mut d = decl();
    d.hooks.truncate(1);
    let args = ["a".to_owned(), "b".to_owned()];
    // Passed out of order: the hash sorts by name.
    let env = [
        ("B".to_owned(), "2".to_owned()),
        ("A".to_owned(), "1".to_owned()),
    ];
    let manifest = format!("sha256:{}", "aa".repeat(32));
    let h = trust_hash(&TrustInputs {
        name: "p",
        tree: &tree,
        declaration: &d,
        program: "tree:bin/run",
        args: &args,
        env: &env,
        path_env: "/usr/bin:/bin",
        manifest_digest: Some(&manifest),
        key_fingerprint: Some("fp"),
        interpreter: None,
        environment: None,
    });
    assert_eq!(
        h.as_str(),
        "sha256:6912f92de3655db4395228d51803e91433919c59dc1fe3eec010b64b6c992946"
    );
}

#[test]
fn a_damaged_store_fails_closed() {
    let s = Scratch::new("corrupt");
    let vp = install(&s, "1.0.0", "");
    let spec = spec_for(&vp, decl());
    let g = gate(&s, &vp);
    trust(&g, &vp, &spec);
    fs::write(s.dir("state/trust.json"), "{not json").unwrap();
    assert!(matches!(
        TrustStore::open(s.dir("state")).records(),
        Err(TrustError::Corrupt { .. })
    ));
    assert!(matches!(g.check(&spec), Err(PluginError::Untrusted(_))));
    let extra = json!({"schema_version": 1, "records": [], "pins": [], "extra": 1});
    fs::write(s.dir("state/trust.json"), extra.to_string()).unwrap();
    assert!(matches!(
        TrustStore::open(s.dir("state")).records(),
        Err(TrustError::Corrupt { .. })
    ));
}

#[test]
fn an_unsupported_schema_version_is_rejected_and_the_file_is_kept() {
    let s = Scratch::new("schema");
    let store = TrustStore::open(s.dir("state"));
    fs::create_dir_all(s.dir("state")).unwrap();
    let file = s.dir("state/trust.json");
    let future = json!({"schema_version": 2, "records": [], "pins": []}).to_string();
    fs::write(&file, &future).unwrap();
    assert!(matches!(store.records(), Err(TrustError::Corrupt { .. })));
    assert!(matches!(
        store.record(record(&Digest::of(b"a").to_string(), "a")),
        Err(TrustError::Corrupt { .. })
    ));
    assert!(matches!(
        store.forget(&Digest::of(b"a")),
        Err(TrustError::Corrupt { .. })
    ));
    assert_eq!(fs::read_to_string(&file).unwrap(), future);
}

#[test]
fn x_keys_are_ignored_at_every_depth_and_other_unknown_keys_still_fail() {
    let s = Scratch::new("xkeys");
    fs::create_dir_all(s.dir("state")).unwrap();
    let file = s.dir("state/trust.json");
    let hash = Digest::of(b"a").to_string();
    let doc = |extra: serde_json::Value| {
        let mut rec = json!({
            "hash": hash, "name": "a", "version": null, "source": "s",
            "key_fingerprint": null,
            "declaration": {"network": false, "hooks": [], "required_features": [],
                            "optional_features": [], "x-inner": {"deep": 1}},
            "recorded_at": NOW, "x-note": "hi"
        });
        rec.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"schema_version": 1, "x-top": [1], "records": [rec], "pins": []})
    };
    let store = TrustStore::open(s.dir("state"));
    fs::write(&file, doc(json!({})).to_string()).unwrap();
    let records = store.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].hash, hash);

    fs::write(&file, doc(json!({"bogus": 1})).to_string()).unwrap();
    assert!(matches!(store.records(), Err(TrustError::Corrupt { .. })));
    // `x` alone and `xx-` are not extension keys.
    fs::write(&file, doc(json!({"x": 1})).to_string()).unwrap();
    assert!(matches!(store.records(), Err(TrustError::Corrupt { .. })));
}

#[test]
fn a_write_that_would_exceed_the_reader_limit_is_refused_and_the_store_is_kept() {
    let s = Scratch::new("toolarge");
    let store = TrustStore::open(s.dir("state"));
    store
        .record(record(&Digest::of(b"small").to_string(), "small"))
        .unwrap();
    let file = s.dir("state/trust.json");
    let before = fs::read(&file).unwrap();
    let huge = "a".repeat(9 << 20);
    let err = store
        .record(record(&Digest::of(b"huge").to_string(), &huge))
        .unwrap_err();
    assert!(matches!(err, TrustError::TooLarge { .. }), "{err:?}");
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(
        store.records().unwrap().len(),
        1,
        "reads and forget still work"
    );
    assert!(store.forget(&Digest::of(b"small")).unwrap());
}

#[test]
fn concurrent_writers_do_not_lose_records() {
    let s = Scratch::new("concurrent");
    let store = TrustStore::open(s.dir("state"));
    std::thread::scope(|scope| {
        for t in 0..8 {
            let store = store.clone();
            scope.spawn(move || {
                for i in 0..5 {
                    store
                        .record(record(
                            &Digest::of(format!("{t}-{i}").as_bytes()).to_string(),
                            &format!("p{t}"),
                        ))
                        .unwrap();
                }
            });
        }
    });
    assert_eq!(store.records().unwrap().len(), 40);
    let first = Digest::of(b"0-0");
    assert!(store.is_trusted(&first).unwrap());
    assert!(store.forget(&first).unwrap());
    assert!(!store.forget(&first).unwrap());
    assert_eq!(store.records().unwrap().len(), 39);
}

#[test]
fn tm22_key_pins_bind_a_namespace_and_feed_the_verifier() {
    let s = Scratch::new("pins");
    let store = TrustStore::open(s.dir("state"));
    let a = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64();
    let b = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64();
    let pin = store.pin_key("acme", &a, NOW).unwrap();
    assert_eq!(pin.fingerprint.len(), 64);
    assert_eq!(
        store.pin_key("acme", &a, NOW).unwrap(),
        pin,
        "pinning the same key again is a no-op"
    );
    let err = store.pin_key("acme", &b, NOW).unwrap_err();
    assert!(matches!(err, TrustError::NamespaceBound { .. }), "{err:?}");
    assert!(store.pin_key("other", &b, NOW).is_ok());
    let pinned = store.pinned_map().unwrap();
    assert_eq!(
        pinned.keys().map(String::as_str).collect::<Vec<_>>(),
        ["acme", "other"]
    );
    assert_eq!(pinned["acme"].len(), 1);
    assert_eq!(pinned["acme"][0].fingerprint(), pin.fingerprint);
    assert!(!pinned.contains_key("nobody"));
    let policy = slopvac_plugins::sign::KeyPolicy {
        pinned,
        ..Default::default()
    };
    assert_eq!(policy.pinned["other"].len(), 1);
    assert!(store.pin_key("acme", "garbage", NOW).is_err());
}

#[cfg(unix)]
mod unix_only {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::common::{cfg, docs, python};
    use slopvac_plugins::extract::tree_hash;
    use slopvac_plugins::{Plugin, StartError};

    /// A spec that starts the fake plugin through the interpreter, which sits
    /// outside the tree and so needs an allowlist entry.
    fn spawn_spec(vp: &VerifiedPlugin) -> PluginSpec {
        PluginSpec {
            program: python(),
            args: vec!["plugin.py".into(), "ok".into(), "{}".into()],
            path_env: "/usr/bin:/bin".into(),
            ..spec_for(vp, decl())
        }
    }

    fn spawn_gate(s: &Scratch, vp: &VerifiedPlugin) -> TrustGate {
        gate(s, vp).with_external_programs([python()])
    }

    #[test]
    fn a_trusted_plugin_starts_and_works() {
        let s = Scratch::new("u-trusted");
        let vp = install(&s, "1.0.0", "");
        let spec = spawn_spec(&vp);
        let g = spawn_gate(&s, &vp);
        trust(&g, &vp, &spec);
        let p = Plugin::start(&spec, &cfg(), &g).unwrap();
        assert_eq!(p.tag("main", &docs(1)).unwrap().len(), 1);
        assert!(p.shutdown().clean());
    }

    #[test]
    fn an_untrusted_plugin_does_not_start() {
        let s = Scratch::new("u-untrusted");
        let vp = install(&s, "1.0.0", "");
        let err = Plugin::start(&spawn_spec(&vp), &cfg(), &spawn_gate(&s, &vp))
            .unwrap_err()
            .error;
        assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
        assert!(err.to_string().contains("slopvac plugins trust"), "{err}");
    }

    #[test]
    fn tm6_an_interpreter_outside_the_tree_does_not_start_without_an_allowlist_entry() {
        let s = Scratch::new("u-external");
        let vp = install(&s, "1.0.0", "");
        let spec = spawn_spec(&vp);
        trust(&spawn_gate(&s, &vp), &vp, &spec);
        let err = Plugin::start(&spec, &cfg(), &gate(&s, &vp))
            .unwrap_err()
            .error;
        assert!(matches!(err, PluginError::Usage(_)), "{err:?}");
    }

    #[test]
    fn tm6_a_bare_program_name_never_reaches_a_path_lookup() {
        let s = Scratch::new("u-bare");
        let vp = install(&s, "1.0.0", "");
        let mut spec = spawn_spec(&vp);
        spec.program = PathBuf::from("python3");
        let err = Plugin::start(&spec, &cfg(), &spawn_gate(&s, &vp))
            .unwrap_err()
            .error;
        assert!(matches!(err, PluginError::Usage(_)), "{err:?}");
    }

    #[test]
    fn tm6_a_symlink_along_the_program_path_is_refused() {
        let s = Scratch::new("u-symlink");
        let vp = install(&s, "1.0.0", "");
        let g = gate(&s, &vp);
        // Added after extraction, so only the program check can see it.
        fs::set_permissions(&vp.root, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink("/bin/sh", vp.root.join("link")).unwrap();
        std::os::unix::fs::symlink(&vp.root, vp.root.join("loop")).unwrap();
        for program in [vp.root.join("link"), vp.root.join("loop/plugin.py")] {
            let mut spec = spec_for(&vp, decl());
            spec.program = program.clone();
            assert!(
                matches!(g.state(&vp, &spec), Err(TrustError::Program(_))),
                "{}",
                program.display()
            );
        }
    }

    #[test]
    fn tm29_slopvac_trust_starts_a_plugin_without_writing_the_store() {
        let s = Scratch::new("u-env");
        let vp = install(&s, "1.0.0", "");
        let spec = spawn_spec(&vp);
        let hash = hash_of(&spawn_gate(&s, &vp), &vp, &spec);
        let g = TrustGate::new(
            TrustStore::open(s.dir("state")),
            vec![hash],
            vec![vp.clone()],
        )
        .with_external_programs([python()]);
        assert!(Plugin::start(&spec, &cfg(), &g).unwrap().shutdown().clean());
        assert!(!s.dir("state").exists());
    }

    #[test]
    fn tm26_a_revoked_plugin_does_not_start() {
        let s = Scratch::new("u-revoked");
        let vp = install(&s, "1.0.0", "");
        let spec = spawn_spec(&vp);
        trust(&spawn_gate(&s, &vp), &vp, &spec);
        let [(_, manifest), (_, key)] = revocations(&vp);
        for rev in [manifest, key] {
            let g = spawn_gate(&s, &vp).with_revoked([rev]);
            assert!(matches!(
                Plugin::start(&spec, &cfg(), &g),
                Err(StartError {
                    error: PluginError::Untrusted(_),
                    ..
                })
            ));
        }
    }

    #[test]
    fn tm6_one_changed_byte_after_trust_stops_the_spawn() {
        let s = Scratch::new("u-tm6");
        let vp = install(&s, "1.0.0", "x");
        let spec = spawn_spec(&vp);
        let g = spawn_gate(&s, &vp);
        trust(&g, &vp, &spec);
        let file = vp.root.join("extra");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, "y").unwrap();
        let now = tree_hash(&vp.root).unwrap();
        assert_ne!(now, vp.tree);
        let err = Plugin::start(&spec, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("slopvac plugins trust"), "{text}");
        assert!(text.contains(now.as_str()), "names the new hash: {text}");
        assert!(
            text.contains(vp.tree.as_str()),
            "names the trusted hash: {text}"
        );
    }

    #[test]
    fn tm6_a_new_executable_bit_after_trust_stops_the_spawn() {
        let s = Scratch::new("u-tm6-exec");
        let vp = install(&s, "1.0.0", "x");
        let spec = spawn_spec(&vp);
        let g = spawn_gate(&s, &vp);
        trust(&g, &vp, &spec);
        fs::set_permissions(vp.root.join("extra"), fs::Permissions::from_mode(0o755)).unwrap();
        let err = Plugin::start(&spec, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
    }

    #[test]
    fn tm6_a_planted_exec_sidecar_after_trust_stops_the_spawn() {
        let s = Scratch::new("u-tm6-sidecar");
        let vp = install(&s, "1.0.0", "x");
        let spec = spawn_spec(&vp);
        let g = spawn_gate(&s, &vp);
        trust(&g, &vp, &spec);
        fs::set_permissions(&vp.root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(vp.root.join(".slopvac-exec"), "extra\n").unwrap();
        let err = Plugin::start(&spec, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
    }

    /// The `(major, minor)` version of `py`. A probe that fails panics: only a
    /// version the probe reads may skip the SDK test.
    fn python_version(py: &Path) -> (u32, u32) {
        let out = std::process::Command::new(py)
            .env_clear()
            .args(["-c", "import sys; print(*sys.version_info[:2])"])
            .output()
            .unwrap_or_else(|e| panic!("running {}: {e}", py.display()));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{} -c failed: {}",
            py.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        let mut parts = text.split_whitespace().map(str::parse::<u32>);
        match (parts.next(), parts.next(), parts.next()) {
            (Some(Ok(major)), Some(Ok(minor)), None) => (major, minor),
            _ => panic!("{} printed an unreadable version: {text:?}", py.display()),
        }
    }

    /// The fence-tagger example from the Python SDK, with the SDK vendored
    /// beside it as a plugin tree would carry it, extracted like a synced plugin.
    fn install_sdk_example(s: &Scratch) -> VerifiedPlugin {
        let pkg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/slopvac-plugin");
        let mut files = vec![(
            "plugin.py".to_owned(),
            fs::read(pkg.join("examples/fence-tagger/plugin.py")).unwrap(),
        )];
        let sdk = pkg.join("src/slopvac_plugin");
        for entry in fs::read_dir(&sdk).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "py") {
                let name = path.file_name().unwrap().to_str().unwrap();
                let data = fs::read(&path).unwrap();
                files.push((format!("slopvac_plugin/{name}"), data));
            }
        }
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let out = extract_targz(
            targz(&refs).as_slice(),
            &s.dir("cache"),
            &HashSet::new(),
            Limits::default(),
        )
        .unwrap();
        VerifiedPlugin {
            name: "fence-tagger".into(),
            version: Some("0.1.0".into()),
            source: "oci:ghcr.io/acme/fence-tagger".into(),
            root: out.root,
            tree: out.hash,
            manifest_digest: Some(Digest::of(b"fence-tagger").to_string()),
            key_fingerprint: Some("e".repeat(64)),
        }
    }

    #[test]
    fn the_python_sdk_example_tags_through_the_host_behind_the_trust_gate() {
        let py = python();
        let version = python_version(&py);
        if version < (3, 11) {
            eprintln!(
                "skipped: {} is Python {}.{}; the SDK needs 3.11 or later",
                py.display(),
                version.0,
                version.1
            );
            return;
        }
        let s = Scratch::new("u-sdk");
        let vp = install_sdk_example(&s);
        let spec = PluginSpec {
            name: vp.name.clone(),
            program: py.clone(),
            // -B: the tree is read-only and must stay byte-identical to what was trusted.
            args: vec!["-B".into(), "plugin.py".into()],
            cwd: vp.root.clone(),
            declaration: PluginDeclaration {
                network: false,
                hooks: vec![HookDecl {
                    kind: "tagger".into(),
                    id: "fences".into(),
                }],
                features: FeatureSet::default(),
            },
            path_env: "/usr/bin:/bin".into(),
            env: vec![],
        };
        let g = gate(&s, &vp).with_external_programs([py]);
        let err = Plugin::start(&spec, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");

        trust(&g, &vp, &spec);
        let p = Plugin::start(&spec, &cfg(), &g).unwrap();
        assert_eq!(p.negotiated().info.name, "fence-tagger");
        let doc = slopvac_plugins::hooks::Document {
            id: "fenced".into(),
            text: "Intro.\n```py\nx = 'é'\n```\nOutro.\n".into(),
            media_type: None,
            path: None,
        };
        let out = p.tag("fences", &[doc]).unwrap();
        let tags = out[0].outcome.as_ref().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].kind, "code");
        // Byte offsets: 7 for "Intro.\n", then 6 + 9 ("é" is two bytes) + 4.
        assert_eq!((tags[0].span.start, tags[0].span.end), (7, 26));
        assert!(p.shutdown().clean());
        g.check(&spec).unwrap();
    }

    #[test]
    fn tm28_the_store_is_private_to_the_user() {
        let s = Scratch::new("u-perms");
        let vp = install(&s, "1.0.0", "");
        let spec = spec_for(&vp, decl());
        trust(&gate(&s, &vp), &vp, &spec);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&s.dir("state")), 0o700);
        assert_eq!(mode(&s.dir("state/trust.json")), 0o600);
    }
}
