//! Output failures exit 2 with a stable code instead of panicking or passing.

use std::io::{self, Write};
use std::process::ExitCode;

use slopvac_cli::env::MapEnv;
use slopvac_cli::main_with_stdout;

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}

#[test]
fn stdout_write_failures_exit_2() {
    let cases: [&[&str]; 4] = [
        &["completions", "bash"],
        &["completions", "zsh"],
        &["man"],
        &["cli-reference"],
    ];
    for args in cases {
        let code = main_with_stdout(
            std::iter::once("slopvac").chain(args.iter().copied()),
            &MapEnv::default(),
            &mut FailingWriter,
        );
        assert_eq!(code, ExitCode::from(2), "{args:?}");
    }
}
