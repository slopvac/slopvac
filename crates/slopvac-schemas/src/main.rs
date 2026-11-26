//! `slopvac-schemas write|check`: regenerate `schemas/v3` from the Rust types, or fail when a
//! committed file differs from what the types generate.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "the binary owns stdout and stderr"
)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Arg, Command, value_parser};

fn command() -> Command {
    let root = Arg::new("root")
        .long("root")
        .value_name("DIR")
        .value_parser(value_parser!(PathBuf))
        .help("Repository root that holds schemas/v3 [default: this checkout]");
    Command::new("slopvac-schemas")
        .about("Generate the schemas/v3 JSON Schemas from the Rust types")
        .subcommand_required(true)
        .subcommand(
            Command::new("write")
                .about("Write the generated files")
                .arg(root.clone()),
        )
        .subcommand(
            Command::new("check")
                .about("Fail when a committed file differs from the generated one")
                .arg(root),
        )
}

fn main() -> ExitCode {
    let matches = command().get_matches();
    let Some((name, sub)) = matches.subcommand() else {
        return ExitCode::from(2);
    };
    let root = sub
        .get_one::<PathBuf>("root")
        .cloned()
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")));
    let outcome = match name {
        "write" => slopvac_schemas::write(&root).map(|written| {
            for file in written {
                println!("wrote {}/{}", file.dir, file.file);
            }
            ExitCode::SUCCESS
        }),
        _ => slopvac_schemas::check(&root).map(|drift| {
            if drift.is_empty() {
                println!("schemas/v3 matches the Rust types");
                return ExitCode::SUCCESS;
            }
            for file in &drift {
                print!("{}", file.diff);
            }
            eprintln!(
                "{} schema file(s) differ from the Rust types; run `just schemas` and commit the result",
                drift.len()
            );
            ExitCode::from(1)
        }),
    };
    outcome.unwrap_or_else(|err| {
        eprintln!("error: {err}");
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        ExitCode::from(2)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_command_tree_is_well_formed() {
        super::command().debug_assert();
    }
}
