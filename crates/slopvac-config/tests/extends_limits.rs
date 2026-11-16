//! `extends` is bounded: diamonds load each file once, and depth, file count and bytes are capped.

use std::fs;
use std::time::{Duration, Instant};

use slopvac_config::{Code, ConfigError, NoPresets, load};

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write(d: &tempfile::TempDir, name: &str, text: &str) {
    fs::write(d.path().join(name), text).unwrap();
}

fn rejected(err: ConfigError) -> slopvac_config::Rejected {
    match err {
        ConfigError::Rejected(r) => *r,
        other @ ConfigError::Io { .. } => panic!("expected a rejection, got {other}"),
    }
}

/// `levels` levels where each file extends the next level's two files, which both extend that
/// level's... (a diamond per level). Without memoisation the loader visits 2^levels files.
fn diamond(d: &tempfile::TempDir, levels: usize) {
    for level in 0..levels {
        for side in ["a", "b"] {
            let name = format!("l{level}{side}.toml");
            let text = if level + 1 == levels {
                "schema_version = 3\n".to_owned()
            } else {
                format!(
                    "schema_version = 3\nextends = [\"l{n}a.toml\", \"l{n}b.toml\"]\n",
                    n = level + 1
                )
            };
            write(d, &name, &text);
        }
    }
    write(
        d,
        "slopvac.toml",
        "schema_version = 3\nextends = [\"l0a.toml\", \"l0b.toml\"]\n",
    );
}

#[test]
fn a_shallow_diamond_loads_each_file_once() {
    let d = dir();
    diamond(&d, 6);
    let config = load(&d.path().join("slopvac.toml"), &NoPresets).unwrap();
    // Two files per level plus the project file; none twice.
    assert_eq!(config.files.len(), 6 * 2 + 1);
}

#[test]
fn a_deep_diamond_stops_at_the_limit_quickly() {
    let d = dir();
    diamond(&d, 20);
    let started = Instant::now();
    let err = load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let r = rejected(err);
    assert_eq!(r.diagnostics[0].code, Code::ExtendsLimit);
}

#[test]
fn a_chain_deeper_than_sixteen_is_refused_and_sixteen_is_allowed() {
    for (len, ok) in [(15, true), (17, false)] {
        let d = dir();
        for i in 0..len {
            let text = if i + 1 == len {
                "schema_version = 3\n".to_owned()
            } else {
                format!("schema_version = 3\nextends = [\"c{}.toml\"]\n", i + 1)
            };
            write(&d, &format!("c{i}.toml"), &text);
        }
        write(
            &d,
            "slopvac.toml",
            "schema_version = 3\nextends = [\"c0.toml\"]\n",
        );
        let result = load(&d.path().join("slopvac.toml"), &NoPresets);
        assert_eq!(result.is_ok(), ok, "depth {len}");
        if let Err(err) = result {
            assert_eq!(rejected(err).diagnostics[0].code, Code::ExtendsLimit);
        }
    }
}

#[test]
fn more_than_sixty_four_distinct_files_are_refused() {
    let d = dir();
    let list: Vec<String> = (0..70).map(|i| format!("\"f{i}.toml\"")).collect();
    for i in 0..70 {
        write(&d, &format!("f{i}.toml"), "schema_version = 3\n");
    }
    write(
        &d,
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", list.join(", ")),
    );
    let r = rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsLimit);
}

#[test]
fn aggregate_bytes_are_capped() {
    let d = dir();
    // Ten files of 500 KiB each: every one is under the per-file limit, together over 4 MiB.
    let pad = format!("schema_version = 3\n#{}\n", "x".repeat(500 * 1024));
    let list: Vec<String> = (0..10).map(|i| format!("\"p{i}.toml\"")).collect();
    for i in 0..10 {
        write(&d, &format!("p{i}.toml"), &pad);
    }
    write(
        &d,
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", list.join(", ")),
    );
    let r = rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsLimit);
}

#[test]
fn a_real_cycle_is_still_a_cycle() {
    let d = dir();
    write(&d, "a.toml", "schema_version = 3\nextends = [\"b.toml\"]\n");
    write(&d, "b.toml", "schema_version = 3\nextends = [\"a.toml\"]\n");
    let r = rejected(load(&d.path().join("a.toml"), &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsCycle);
}
