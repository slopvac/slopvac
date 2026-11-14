//! Plugin discovery, execution and distribution.
//!
//! The plugin host speaks JSON-RPC 2.0 over stdio with Content-Length
//! framing. Its contract is `docs/contract/plugin-protocol.md`. The host runs
//! no async runtime: it uses one set of threads per plugin and
//! `crossbeam-channel` deadlines. Threat-model controls are in
//! `docs/contract/threat-model.md`. The OCI client is `oci`, and the trust
//! store lands in its own module.

pub mod api;
pub mod error;
pub mod frame;
pub mod hooks;
pub mod host;
pub mod oci;
pub mod process;
pub mod protocol;
pub mod strict;

pub use api::{DocOutcome, plan_batches};
pub use error::{PluginError, RpcError, StartError};
pub use host::{HostConfig, Plugin, PluginSpec, RunCaps, Shutdown, SpawnGate, Timeouts};
pub use process::{Teardown, kill_all_live};
pub use protocol::{HookKind, PluginDeclaration};
