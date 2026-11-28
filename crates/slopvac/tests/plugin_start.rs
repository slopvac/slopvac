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

/// A rewrite, so the review asks the provider.
const REWRITE: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    "# Cache\n\nThe cache holds 20 entries. It evicts the least recently used entry first. Reads refresh an entry.\n",
);

/// The trusted plugin runs under the launcher's limits, the approved defaults of threat model
/// section 5.5: 30 minutes of CPU time with the kill a second later, no core files, 1,024 open
/// files and 256 MiB per file.
#[test]
fn review_starts_a_trusted_plugin_under_the_launchers_limits() {
    let world = World::new(REWRITE, &configured());
    world.trust();

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(world.spawns(), 1);
    assert_eq!(world.limits("cpu"), (Some(1800), Some(1801)));
    assert_eq!(world.limits("core"), (Some(0), Some(0)));
    assert_eq!(world.limits("nofile"), (Some(1024), Some(1024)));
    assert_eq!(world.limits("fsize"), (Some(256 << 20), Some(256 << 20)));
}
