//! `slopvac onboard` and the pointer for the removed `slopvac reference`
//! (compat rows `cmd:onboard`, `cmd:reference`; bead slopvac-8l4.12.10).

use clap::Command;

/// The text `slopvac onboard` prints.
pub const TEXT: &str = "\
# Slopvac agent setup

Run `slopvac init` to write a starter `slopvac.toml`. Run `slopvac setup --list` to
name the supported agent harnesses, and `slopvac setup <harness>` to install steering
and the `slopvac-prose` skill into an existing project. The managed block stays small.

Run `slopvac prime` whenever you need the current detailed lint workflow.
";

/// What `slopvac reference` prints. 3.0 has no compatibility mode, so the command only points
/// at its replacement.
pub const REFERENCE_REMOVED: &str = "`slopvac reference` is removed in 3.0. The rules reference is generated with the docs site: run `just docs` to write it and `just docs-check` to check it in CI.";

/// The `onboard` subcommand.
#[must_use]
pub fn command() -> Command {
    Command::new("onboard").about("Print the agent onboarding text.")
}
