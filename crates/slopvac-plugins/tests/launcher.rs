//! The plugin launcher, run directly as the host runs it: limits under an
//! inherited ceiling, and refusals of malformed invocations.
#![cfg(unix)]

mod common;

use std::process::{Command, Output};

use slopvac_plugins::launch::FLAG;

fn launcher() -> &'static str {
    env!("CARGO_BIN_EXE_slopvac-plugin-launcher")
}

/// Run the launcher under `sh`, after `prelude`, with `args` after the flag.
fn run_launcher(prelude: &str, args: &[&str]) -> Output {
    let script = format!("{prelude}\nexec \"$0\" \"$@\"");
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(launcher())
        .arg(FLAG)
        .args(args)
        .env_clear()
        .output()
        .unwrap()
}

#[test]
fn the_launcher_never_raises_a_hard_limit_already_in_place() {
    let python = common::python();
    let report =
        "import resource as r; print(r.getrlimit(r.RLIMIT_NOFILE), r.getrlimit(r.RLIMIT_CPU))";
    let out = run_launcher(
        // Soft and hard: below what the launcher asks for.
        "ulimit -n 32 && ulimit -t 50",
        &[
            "cpu=100",
            "nofile=64",
            "fsize=1048576",
            "data=536870912",
            "sweep=2000",
            "--",
            python.to_str().unwrap(),
            "-c",
            report,
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout.trim(), "(32, 32) (50, 50)", "{out:?}");
}

#[test]
fn a_malformed_launcher_invocation_runs_nothing_and_exits_127() {
    let dir = common::scratch("launcher-malformed");
    let marker = dir.join("ran");
    let m = marker.to_str().unwrap();
    let ok = [
        "cpu=5",
        "nofile=64",
        "fsize=10",
        "data=20",
        "sweep=0",
        "--",
        "/usr/bin/touch",
        m,
    ];
    // Each case breaks one part of a valid invocation.
    let cases: Vec<Vec<&str>> = vec![
        ok[..6].to_vec(),                      // no program
        [&ok[..5], &["-"], &ok[6..]].concat(), // no `--`
        [&["cpu=+5"], &ok[1..]].concat(),      // a sign
        [&["cpu="], &ok[1..]].concat(),        // no digits
        [&[ok[1], ok[0]], &ok[2..]].concat(),  // out of order
        [&ok[..6], &["touch", m]].concat(),    // a relative program
    ];
    let mut failures = Vec::new();
    for args in &cases {
        let out = run_launcher(":", args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.code() != Some(127)
            || !stderr.starts_with("slopvac plugin launcher: ")
            || marker.exists()
        {
            failures.push(format!("{args:?}: {out:?}"));
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(failures.is_empty(), "{failures:#?}");
}
