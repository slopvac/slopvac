//! Plugin host for slopvac: JSON-RPC 2.0 over stdio with Content-Length framing.
//!
//! The contract is `docs/contract/plugin-protocol.md`. The host runs no async
//! runtime: it uses one set of threads per plugin and `crossbeam-channel`
//! deadlines. Threat-model controls are in `docs/contract/threat-model.md`.

pub mod api;
pub mod error;
pub mod frame;
pub mod hooks;
pub mod host;
pub mod process;
pub mod protocol;
pub mod strict;

pub use api::{DocOutcome, plan_batches};
pub use error::{PluginError, RpcError};
pub use host::{HostConfig, Plugin, PluginSpec, RunCaps, SpawnGate, Timeouts};
pub use process::kill_all_live;
pub use protocol::{HookKind, PluginDeclaration};
