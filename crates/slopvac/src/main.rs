//! The `slopvac` binary.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the binary owns stdout and stderr"
)]

fn main() -> std::process::ExitCode {
    // First thing, before arguments, environment or config: the plugin host starts this
    // executable as its launcher, and the launcher mode must run before anything else.
    slopvac_plugins::launch::intercept_for_users();
    slopvac_cli::main_with(std::env::args_os(), &slopvac_cli::SystemEnv)
}
