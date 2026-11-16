//! `extends` is bounded: diamonds load each file once, and depth, file count and bytes are capped.

use std::fmt::Write as _;
use std::fs;
use std::time::{Duration, Instant};

use slopvac_config::{Code, ConfigError, Inputs, NoPresets, load};

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

fn resolved(path: &std::path::Path) -> slopvac_config::Resolved {
    let config = load(path, &NoPresets).unwrap();
    config.resolve(
        std::path::Path::new("doc.md"),
        &Inputs {
            presets: &NoPresets,
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        },
    )
}

#[test]
fn a_later_entry_wins_even_when_an_earlier_entry_reaches_the_same_file() {
    // project extends A then B, and A extends B. A sets update_check = true, B sets false.
    // B is the later entry, so the effective value is false (docs/contract/config.md, 2.1).
    let d = dir();
    write(
        &d,
        "b.toml",
        "schema_version = 3\nlocale = \"en-GB\"\n[network]\nupdate_check = false\n[thresholds]\nmax_warnings = 2\n",
    );
    write(
        &d,
        "a.toml",
        "schema_version = 3\nextends = [\"b.toml\"]\nlocale = \"en-US\"\n[network]\nupdate_check = true\n[thresholds]\nmax_warnings = 7\n",
    );
    write(
        &d,
        "slopvac.toml",
        "schema_version = 3\nextends = [\"a.toml\", \"b.toml\"]\n",
    );
    let r = resolved(&d.path().join("slopvac.toml"));
    assert_eq!(
        r.get(&["network", "update_check"]),
        Some(&serde_json::json!(false))
    );
    assert_eq!(r.get(&["locale"]), Some(&serde_json::json!("en-GB")));
    assert_eq!(
        r.get(&["thresholds", "max_warnings"]),
        Some(&serde_json::json!(2))
    );

    // The other way round, A is the later entry and wins.
    write(
        &d,
        "slopvac.toml",
        "schema_version = 3\nextends = [\"b.toml\", \"a.toml\"]\n",
    );
    let r = resolved(&d.path().join("slopvac.toml"));
    assert_eq!(
        r.get(&["network", "update_check"]),
        Some(&serde_json::json!(true))
    );
    assert_eq!(r.get(&["locale"]), Some(&serde_json::json!("en-US")));
    assert_eq!(
        r.get(&["thresholds", "max_warnings"]),
        Some(&serde_json::json!(7))
    );
}

#[test]
fn the_project_still_beats_every_extended_file_and_override_blocks_keep_file_order() {
    let d = dir();
    write(
        &d,
        "b.toml",
        "schema_version = 3\n[[overrides]]\nfiles = [\"*.md\"]\n[overrides.thresholds]\nmax_warnings = 1\n",
    );
    write(
        &d,
        "a.toml",
        "schema_version = 3\nextends = [\"b.toml\"]\n[[overrides]]\nfiles = [\"doc.md\"]\n[overrides.thresholds]\nmax_warnings = 5\n",
    );
    write(
        &d,
        "slopvac.toml",
        "schema_version = 3\nextends = [\"a.toml\", \"b.toml\"]\n[thresholds]\nmax_warnings = 9\n",
    );
    let r = resolved(&d.path().join("slopvac.toml"));
    // Blocks of a later file apply after blocks of an earlier one; B is last, so B's block wins
    // over A's for doc.md, and both beat the project's whole-file value.
    assert_eq!(
        r.get(&["thresholds", "max_warnings"]),
        Some(&serde_json::json!(1))
    );
}

/// `n` files `f0..f{n-1}` where each extends the next, plus the project: a long chain.
fn star(d: &tempfile::TempDir, n: usize) {
    let list: Vec<String> = (0..n).map(|i| format!("\"f{i}.toml\"")).collect();
    for i in 0..n {
        write(d, &format!("f{i}.toml"), "schema_version = 3\n");
    }
    write(
        d,
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", list.join(", ")),
    );
}

#[test]
fn exactly_sixty_four_files_including_the_project_load_and_sixty_five_do_not() {
    let d = dir();
    star(&d, 63);
    assert_eq!(
        load(&d.path().join("slopvac.toml"), &NoPresets)
            .unwrap()
            .files
            .len(),
        64
    );
    let d = dir();
    star(&d, 64);
    assert_eq!(
        rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err()).diagnostics[0].code,
        Code::ExtendsLimit
    );
}

/// A project that extends a chain `c0..c9` (each extends the next) where every chain file also
/// extends some leaves, `leaves` in total. Distinct files: 1 + 10 + `leaves`.
fn branching(d: &tempfile::TempDir, leaves: usize) {
    let mut per_level = [leaves / 10; 10];
    for slot in per_level.iter_mut().take(leaves % 10) {
        *slot += 1;
    }
    let mut next = 0;
    for (i, count) in per_level.iter().enumerate() {
        let mut entries = Vec::new();
        if i + 1 < 10 {
            entries.push(format!("\"c{}.toml\"", i + 1));
        }
        for _ in 0..*count {
            entries.push(format!("\"l{next}.toml\""));
            write(d, &format!("l{next}.toml"), "schema_version = 3\n");
            next += 1;
        }
        write(
            d,
            &format!("c{i}.toml"),
            &format!("schema_version = 3\nextends = [{}]\n", entries.join(", ")),
        );
    }
    write(
        d,
        "slopvac.toml",
        "schema_version = 3\nextends = [\"c0.toml\"]\n",
    );
}

#[test]
fn unfinished_ancestors_hold_a_slot_in_a_branching_graph() {
    // 1 project + 10 chain files + 53 leaves = 64 loads; one more leaf is 65 and is refused.
    let d = dir();
    branching(&d, 53);
    assert_eq!(
        load(&d.path().join("slopvac.toml"), &NoPresets)
            .unwrap()
            .files
            .len(),
        64
    );
    let d = dir();
    branching(&d, 54);
    assert_eq!(
        rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err()).diagnostics[0].code,
        Code::ExtendsLimit
    );
}

#[test]
fn the_project_file_counts_toward_the_byte_budget() {
    let d = dir();
    // Seven extended files of 500 KiB are 3.5 MiB: under 4 MiB alone, over with a 600 KiB project.
    let pad = format!("schema_version = 3\n#{}\n", "x".repeat(500 * 1024));
    let list: Vec<String> = (0..7).map(|i| format!("\"p{i}.toml\"")).collect();
    for i in 0..7 {
        write(&d, &format!("p{i}.toml"), &pad);
    }
    write(
        &d,
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", list.join(", ")),
    );
    assert!(load(&d.path().join("slopvac.toml"), &NoPresets).is_ok());
    let big = format!(
        "schema_version = 3\nextends = [{}]\n#{}\n",
        list.join(", "),
        "y".repeat(600 * 1024)
    );
    write(&d, "slopvac.toml", &big);
    assert_eq!(
        rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err()).diagnostics[0].code,
        Code::ExtendsLimit
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_alias_is_the_same_file_and_costs_no_slot() {
    let d = dir();
    write(&d, "real.toml", "schema_version = 3\n");
    let list: Vec<String> = (0..70).map(|i| format!("\"alias{i}.toml\"")).collect();
    for i in 0..70 {
        std::os::unix::fs::symlink(
            d.path().join("real.toml"),
            d.path().join(format!("alias{i}.toml")),
        )
        .unwrap();
    }
    write(
        &d,
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", list.join(", ")),
    );
    // 70 names, one file: the project and real.toml.
    assert_eq!(
        load(&d.path().join("slopvac.toml"), &NoPresets)
            .unwrap()
            .files
            .len(),
        2
    );
}

#[test]
fn thirty_thousand_blocks_are_refused_fast() {
    let d = dir();
    let mut text = String::from("schema_version = 3\n");
    for i in 0..30_000 {
        let _ = writeln!(text, "[[overrides]]\nfiles = [\"{i}\"]");
    }
    assert!(text.len() < 1024 * 1024, "{}", text.len());
    write(&d, "slopvac.toml", &text);
    let started = Instant::now();
    let r = rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(r.diagnostics[0].code, Code::ExtendsLimit);
}

#[test]
fn a_thousand_blocks_load_and_a_block_with_too_many_patterns_is_refused() {
    let d = dir();
    let mut text = String::from("schema_version = 3\n");
    for i in 0..1000 {
        let _ = writeln!(text, "[[overrides]]\nfiles = [\"{i}\"]");
    }
    write(&d, "slopvac.toml", &text);
    let started = Instant::now();
    assert_eq!(
        load(&d.path().join("slopvac.toml"), &NoPresets)
            .unwrap()
            .files[0]
            .blocks
            .len(),
        1000
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );

    let globs: Vec<String> = (0..101).map(|i| format!("\"g{i}\"")).collect();
    write(
        &d,
        "slopvac.toml",
        &format!(
            "schema_version = 3\n[[overrides]]\nfiles = [{}]\n",
            globs.join(", ")
        ),
    );
    let r = rejected(load(&d.path().join("slopvac.toml"), &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsLimit);
}
