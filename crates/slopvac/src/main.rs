//! The `slopvac` binary.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the binary owns stdout and stderr"
)]

fn main() -> std::process::ExitCode {
    slopvac_cli::main_with(std::env::args_os(), &slopvac_cli::SystemEnv)
}
