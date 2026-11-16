//! The workspace message registry as one unit: valid entries, a lock file that only grows,
//! and a Fluent export that covers every code.

use std::collections::BTreeMap;

use slopvac_cli::messages::{ALL, RETIRED, catalogs};
use slopvac_messages::{audit, to_ftl};

const LOCK: &str = include_str!("../../../docs/reference/error-codes.lock");

/// The lock as code to kind. Fails the test on a line that is not `code kind`.
fn locked() -> BTreeMap<&'static str, &'static str> {
    LOCK.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split_once(' ')
                .unwrap_or_else(|| panic!("bad lock line {l:?}"))
        })
        .collect()
}

fn registered() -> BTreeMap<&'static str, &'static str> {
    catalogs()
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|e| (e.code, e.kind.name()))
        .collect()
}

#[test]
fn the_registry_passes_the_audit() {
    let problems: Vec<String> = audit(&catalogs()).iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn every_registered_code_is_in_the_lock_with_its_kind() {
    let locked = locked();
    for (code, kind) in registered() {
        assert_eq!(
            locked.get(code),
            Some(&kind),
            "add `{code} {kind}` to docs/reference/error-codes.lock; never change a locked kind"
        );
    }
}

#[test]
fn every_locked_code_is_registered_or_retired() {
    let registered = registered();
    for code in locked().keys() {
        let retired = RETIRED.iter().any(|r| r.code == *code);
        assert!(
            registered.contains_key(code) || retired,
            "`{code}` left the catalogs; list it in RETIRED instead of deleting it"
        );
    }
}

#[test]
fn a_retired_code_is_locked_and_no_longer_registered() {
    let (locked, registered) = (locked(), registered());
    for r in RETIRED {
        assert!(
            locked.contains_key(r.code),
            "`{}` is not in the lock",
            r.code
        );
        assert!(
            !registered.contains_key(r.code),
            "`{}` is retired but still registered",
            r.code
        );
    }
}

#[test]
fn the_lock_is_sorted_and_has_no_duplicates() {
    let codes: Vec<&str> = LOCK
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(' ').next().unwrap_or_default())
        .collect();
    assert!(
        codes.windows(2).all(|w| w[0] < w[1]),
        "keep the lock sorted by code"
    );
}

#[test]
fn the_fluent_export_has_one_message_per_registered_code() {
    let entries: Vec<_> = ALL.iter().flat_map(|(_, f)| f().iter().copied()).collect();
    let ftl = to_ftl(&entries);
    let ids: Vec<&str> = ftl
        .lines()
        .filter(|l| !l.starts_with(['#', ' ']) && !l.is_empty())
        .map(|l| l.split(" = ").next().unwrap_or_default())
        .collect();
    let codes: Vec<&str> = entries.iter().map(|e| e.code).collect();
    assert_eq!(ids, codes);
}
