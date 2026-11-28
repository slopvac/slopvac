//! A code plugin started by a real command of the installed binary, through the default
//! launcher.
//!
//! The host starts every Unix plugin through a launcher: the running executable, run again with
//! `--slopvac-plugin-launcher`, which sets the plugin's resource limits and then runs it. Here
//! `slopvac review` starts a trusted path-source provider plugin from `review_plugin`, and the
//! plugin reports the limits it runs under. Nothing in the binary is replaced.
//!
//! `review_provider.rs` covers the rest of this path through the same binary and fixture. These
//! tests cover that the plugin's result reaches the report
//! (`a_complete_reply_becomes_the_report_in_payload_order`), and that an untrusted plugin exits
//! 2 with `plugin-untrusted` and never starts (`each_gate_alone_keeps_the_provider_from_starting`,
//! case `untrusted`). The launcher that is not installed is a host test in `slopvac-plugins`
//! (`launcher_not_installed.rs`), because the binary always installs it.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod review_plugin;

use review_plugin::{World, configured, stderr};
use rustix::process::{Resource, getrlimit};

/// A rewrite, so the review asks the provider.
const REWRITE: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    "# Cache\n\nThe cache holds 20 entries. It evicts the least recently used entry first. Reads refresh an entry.\n",
);

/// The launcher's limits, the approved defaults of threat model section 5.5, as the name the
/// plugin logs, the resource, and the soft and hard values: 30 minutes of CPU time with the
/// kill a second later, no core files, 1,024 open files and 256 MiB per file.
const LAUNCHER: [(&str, Resource, u64, u64); 4] = [
    ("cpu", Resource::Cpu, 1800, 1801),
    ("core", Resource::Core, 0, 0),
    ("nofile", Resource::Nofile, 1024, 1024),
    ("fsize", Resource::Fsize, 256 << 20, 256 << 20),
];

/// `value`, lowered to the inherited hard limit `hard` (`None`: unlimited, above every value).
fn capped(value: u64, hard: Option<u64>) -> u64 {
    hard.map_or(value, |h| value.min(h))
}

/// The trusted plugin runs under the launcher's limits. The launcher cannot raise a hard limit
/// that the test process already inherited, so each value it sets is the lesser of its own and
/// the inherited hard limit. At least one inherited hard limit must lie above the launcher's,
/// or the plugin's limits would look the same without the launcher.
#[test]
fn review_starts_a_trusted_plugin_under_the_launchers_limits() {
    let inherited: Vec<Option<u64>> = LAUNCHER
        .iter()
        .map(|(_, resource, _, _)| getrlimit(*resource).maximum)
        .collect();
    let telling: Vec<&str> = LAUNCHER
        .iter()
        .zip(&inherited)
        .filter(|((_, _, _, hard), inherited)| inherited.is_none_or(|h| h > *hard))
        .map(|((name, ..), _)| *name)
        .collect();
    assert!(
        !telling.is_empty(),
        "the inherited hard limits of cpu, core, nofile and fsize ({inherited:?}) are all at or \
         below the launcher's, so this run cannot tell that the launcher set them"
    );
    let world = World::new(REWRITE, &configured());
    world.trust();

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(world.spawns(), 1);
    for ((name, _, soft, hard), inherited) in LAUNCHER.iter().zip(&inherited) {
        let expected = (
            Some(capped(*soft, *inherited)),
            Some(capped(*hard, *inherited)),
        );
        assert_eq!(world.limits(name), expected, "the {name} limit");
    }
}
