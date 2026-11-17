//! Plugin discovery, execution and distribution.
//!
//! The plugin host speaks JSON-RPC 2.0 over stdio with Content-Length
//! framing. Its contract is `docs/contract/plugin-protocol.md`. The host runs
//! no async runtime: it uses one set of threads per plugin and
//! `crossbeam-channel` deadlines. Threat-model controls are in
//! `docs/contract/threat-model.md`.
//!
//! The other modules: the OCI client ([`oci`]), the `.slopvac/packs.lock`
//! model ([`lock`]: strict reader, canonical writer, drift diff and the
//! `outdated` query), [`sync`] (install the lock, status, CI check), the pack
//! [`index`] reader, builder and search, the [`trust`] store and spawn
//! gate, the signed revocation list with its cache ([`revoke`]), and
//! managed Python environments for plugins ([`pyenv`]).

pub mod api;
pub mod apply;
pub mod budget;
pub mod error;
pub mod extract;
pub mod frame;
pub mod hooks;
pub mod host;
pub mod index;
pub mod lock;
mod num;
pub mod oci;
pub mod process;
pub mod protocol;
pub mod pyenv;
pub mod revoke;
pub mod sign;
pub mod strict;
pub mod sync;
pub mod trust;

pub use api::{DocOutcome, plan_batches};
pub use budget::{BudgetCaps, RunBudget};
pub use error::{PluginError, RpcError, StartError};
pub use host::{HostConfig, Plugin, PluginSpec, ReadHook, RunCaps, Shutdown, SpawnGate, Timeouts};
pub use process::{Teardown, kill_all_live};
pub use protocol::{HookKind, PluginDeclaration};
