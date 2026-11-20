//! Update notices, install-method detection and `slopvac self update`.
//!
//! Contract: `docs/contract/policies.md`, sections 1.2 and 1.3.
//!
//! - [`notice`]: the 24-hour cache, the policy that decides whether a check may run, and the
//!   [`Notice`](notice::Notice) that `prime`, JSON `notices` and `--version --json` show.
//! - [`install`]: which channel installed this binary, and so which command upgrades it.
//! - [`policy`]: the project `[network]` settings (a stub until the config layer lands).
//! - [`pipeline`]: lock, download, verify, extract, replace. Verification is its own step
//!   ([`verify::ArtifactVerifier`]) and runs before anything touches the installed binary.
//! - [`messages`]: the message catalog for the codes that the update lock and the installed
//!   binary report.
//! - [`command`]: the clap subtree and the `self update` entry point.
//!
//! The crate uses blocking `ureq` and one worker thread with a deadline; there is no async runtime.
//! It does not use `axoupdater`: that crate downloads and runs the installer script itself, so
//! there is no point between download and replace where the minisign signature could be checked.

mod bounded;
pub mod command;
pub mod fetch;
pub mod install;
mod lock;
pub mod messages;
pub mod notice;
pub mod pipeline;
pub mod policy;
mod probe;
pub mod verify;

pub use messages::{error_entries, phrase_entries, text_entries};

/// GitHub repository that publishes releases.
pub const REPO: &str = "slopvac/slopvac";
