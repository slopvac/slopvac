//! The workspace message registry as one unit: valid entries, a lock file that only grows,
//! and a Fluent export that covers every code.

use slopvac_cli::messages::{ALL, RETIRED, Retired, catalogs, check_lock};
use slopvac_messages::{Entry, Kind, audit, to_ftl};

const LOCK: &str = include_str!("../../../docs/reference/error-codes.lock");

const fn entry(code: &'static str, kind: Kind) -> Entry {
    Entry::new(code, kind, "E::V", "text", &[], "Summary.")
}

#[rstest::rstest]
#[case::in_progress("update-in-progress")]
#[case::lock_unsafe("update-lock-unsafe")]
#[case::interrupted("update-interrupted")]
#[case::version_unknown("update-version-unknown")]
fn a_self_update_refusal_code_is_registered_as_an_error(#[case] code: &str) {
    let entry = catalogs()
        .into_iter()
        .flat_map(|(_, entries)| entries.iter())
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("`{code}` is not in the registry"));
    assert_eq!(entry.kind, Kind::Error);
}

#[test]
fn the_registry_passes_the_audit() {
    let problems: Vec<String> = audit(&catalogs()).iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_committed_lock_matches_the_registry() {
    assert_eq!(check_lock(LOCK, &catalogs(), RETIRED), Vec::<String>::new());
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

mod lock_rules {
    use super::*;

    const ENTRIES: [Entry; 2] = [entry("alpha", Kind::Error), entry("beta", Kind::Notice)];

    fn check(lock: &str, retired: &[Retired]) -> Vec<String> {
        check_lock(lock, &[("cat", &ENTRIES)], retired)
    }

    #[test]
    fn a_lock_that_lists_every_code_with_its_kind_is_clean() {
        assert_eq!(
            check("# comment\nalpha error\nbeta notice\n", &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_registered_code_missing_from_the_lock_is_reported() {
        assert_eq!(
            check("alpha error\n", &[]),
            ["`beta` is not in the lock; add `beta notice`"]
        );
    }

    #[test]
    fn a_lock_entry_with_the_wrong_kind_is_reported() {
        assert_eq!(
            check("alpha error\nbeta error\n", &[]),
            ["`beta` is locked as error but registered as notice; a code never changes kind"]
        );
    }

    #[test]
    fn a_code_removed_without_retirement_is_reported() {
        assert_eq!(
            check("alpha error\nbeta notice\ngamma error\n", &[]),
            ["`gamma` left the catalogs; list it in RETIRED instead of deleting it"]
        );
    }

    #[test]
    fn a_retired_code_is_accepted_while_it_stays_locked() {
        let retired = [Retired {
            code: "gamma",
            note: "Gone.",
        }];
        assert_eq!(
            check("alpha error\nbeta notice\ngamma error\n", &retired),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_retired_code_missing_from_the_lock_is_reported() {
        let retired = [Retired {
            code: "gamma",
            note: "Gone.",
        }];
        assert_eq!(
            check("alpha error\nbeta notice\n", &retired),
            ["retired `gamma` is not in the lock"]
        );
    }

    #[test]
    fn a_retired_code_that_is_registered_again_is_reported() {
        let retired = [Retired {
            code: "beta",
            note: "Gone.",
        }];
        assert_eq!(
            check("alpha error\nbeta notice\n", &retired),
            ["retired `beta` is registered again; never reuse a code"]
        );
    }

    #[test]
    fn a_duplicate_lock_entry_is_reported() {
        assert_eq!(
            check("alpha error\nalpha error\nbeta notice\n", &[]),
            ["`alpha` is locked twice"]
        );
    }

    #[test]
    fn an_unsorted_lock_is_reported() {
        assert_eq!(
            check("beta notice\nalpha error\n", &[]),
            ["the lock is unsorted: `beta` before `alpha`"]
        );
    }

    #[test]
    fn a_line_without_a_kind_is_reported() {
        assert_eq!(
            check("alpha\nbeta notice\n", &[]),
            [
                "lock line \"alpha\" is not `code kind`",
                "`alpha` is not in the lock; add `alpha error`"
            ]
        );
    }
}
