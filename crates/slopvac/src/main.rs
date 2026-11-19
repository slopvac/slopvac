//! The `slopvac` binary.

fn main() -> std::process::ExitCode {
    // The plugin host starts every plugin through this executable with a launcher flag. The
    // interception must come first, before clap, the environment or any configuration.
    slopvac_plugins::launch::intercept();
    slopvac_cli::main_with(std::env::args_os(), &slopvac_cli::SystemEnv)
}
