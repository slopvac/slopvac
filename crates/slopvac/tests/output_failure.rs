//! Output failures exit 2 with a stable code instead of panicking or passing.
#![cfg(test)]

use std::io::{self, Write};
use std::process::ExitCode;

use slopvac_cli::env::MapEnv;
use slopvac_cli::main_with_io;

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}

fn run(args: &[&str], out: &mut dyn Write) -> (ExitCode, String) {
    let mut err = Vec::new();
    let code = main_with_io(
        std::iter::once("slopvac").chain(args.iter().copied()),
        &MapEnv::default(),
        out,
        &mut err,
    );
    (code, String::from_utf8(err).expect("utf-8 stderr"))
}

#[test]
fn stdout_write_failures_exit_2_with_a_code() {
    let cases: [&[&str]; 6] = [
        &["completions", "bash"],
        &["completions", "zsh"],
        &["man"],
        &["cli-reference"],
        &["--help"],
        &["--version"],
    ];
    for args in cases {
        let (code, stderr) = run(args, &mut FailingWriter);
        assert_eq!(code, ExitCode::from(2), "{args:?}");
        assert!(
            stderr.starts_with("error[output-failed]: "),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn help_and_version_go_through_the_supplied_writer() {
    for (args, needle) in [
        (&["--help"][..], "Usage: slopvac"),
        (&["--version"][..], "slopvac "),
    ] {
        let mut out = Vec::new();
        let (code, stderr) = run(args, &mut out);
        assert_eq!(code, ExitCode::SUCCESS, "{args:?}");
        assert!(String::from_utf8(out).unwrap().contains(needle), "{args:?}");
        assert!(stderr.is_empty(), "{args:?}: {stderr}");
    }
}

/// Accepts writes but fails on flush, as a closed pipe can after buffering.
struct FlushFails;

impl Write for FlushFails {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}

#[test]
fn lint_report_write_and_flush_failures_exit_2_with_a_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let doc = dir.path().join("doc.md");
    std::fs::write(&doc, "one two\n").expect("write");
    let doc = doc.to_str().expect("utf-8 path");
    for format in ["text", "json"] {
        let args = ["lint", doc, "--format", format];
        let (code, stderr) = run(&args, &mut FailingWriter);
        assert_eq!(code, ExitCode::from(2), "write {format}");
        assert!(stderr.starts_with("error[output-failed]: "), "{stderr}");
        let (code, stderr) = run(&args, &mut FlushFails);
        assert_eq!(code, ExitCode::from(2), "flush {format}");
        assert!(stderr.starts_with("error[output-failed]: "), "{stderr}");
    }
}
