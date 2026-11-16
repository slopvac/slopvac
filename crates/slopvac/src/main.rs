//! The `slopvac` binary.

fn main() -> std::process::ExitCode {
    slopvac_cli::main_with(std::env::args_os(), &slopvac_cli::SystemEnv)
}
