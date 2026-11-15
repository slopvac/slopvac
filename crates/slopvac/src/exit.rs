//! Process exit codes shared by every command.

use std::process::ExitCode;

/// The run finished and no gate failed.
pub const PASS: u8 = 0;
/// The run finished and at least one gate failed.
pub const GATE_FAILED: u8 = 1;
/// The run was incomplete or the invocation was invalid.
pub const INCOMPLETE: u8 = 2;

/// Convert a code to a process exit status.
#[must_use]
pub fn code(value: u8) -> ExitCode {
    ExitCode::from(value)
}
