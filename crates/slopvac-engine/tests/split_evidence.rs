//! A tag whose evidence is long, and that is split into many pieces, costs memory in
//! proportion to the document, not to the evidence times the pieces.
//!
//! The document is an RST simple table. Its last column holds one sentence that says
//! "no" before a very long path, and then runs over many short continuation lines. The
//! middle column wraps beside it, so the sentence's tag is split into one piece for each
//! line. The test counts the growth of the process's peak memory over the lint, not the time. Peak
//! memory is per process, and nextest gives each test its own.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(target_os = "linux")]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use serde_json::json;
use slopvac_engine::Input;

/// The most memory the process has held at once, in bytes, from `VmHWM` in `/proc`.
fn peak_bytes() -> Result<usize, Box<dyn std::error::Error>> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let kib = status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().strip_suffix("kB"))
        .ok_or("no VmHWM line in /proc/self/status")?;
    Ok(kib.trim().parse::<usize>()? * 1024)
}

const PATH_LEN: usize = 60_000;
const CONTINUATIONS: usize = 10_000;

fn source() -> String {
    let path = format!("/{}", "a".repeat(PATH_LEN));
    let mut s = String::from("=  =  ====\n");
    s.push_str(&format!("A  m  It has no {path}\n"));
    for _ in 0..CONTINUATIONS {
        s.push_str("   m  a\n");
    }
    s.push_str("=  =  ====\n");
    s
}

fn driver_output(last: usize) -> String {
    let middle = vec!["m"; CONTINUATIONS + 1].join("\\n");
    let tail = vec!["a"; CONTINUATIONS].join("\\n");
    let cell = |column: usize, text: &str| {
        format!(
            r#"{{"tag":"paragraph","start":2,"end":{last},"containers":[],"cell":[0,{column},1],"pieces":[{{"k":"t","t":"{text}"}}]}}"#
        )
    };
    format!(
        r#"{{"docutils":"0.21.2","nodes":[{{"tag":"table","table":true,"start":1,"end":{},"containers":[]}},{},{}]}}"#,
        last + 1,
        cell(1, &middle),
        cell(2, &format!("It has no /{}\\n{tail}", "a".repeat(PATH_LEN))),
    )
}

#[test]
fn the_pieces_of_a_split_tag_share_its_long_evidence() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt as _;
    let source = source();
    let last = CONTINUATIONS + 2;
    let dir = common::private_dir("rst-split-evidence");
    let json_path = dir.join("out.json");
    std::fs::write(&json_path, driver_output(last))?;
    let python = dir.join("python");
    std::fs::write(
        &python,
        format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", json_path.display()),
    )?;
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755))?;
    let mut options = slopvac_engine::rst::RstOptions::default();
    options.interpreter = Some(python);
    let gate = json!({"contexts": {"only": ["requirement"]}});
    let engine = common::engine(&[common::tokens("dull", "has", &gate)]).with_rst(options);

    let before = peak_bytes()?;
    let result = engine.lint(&Input::rst("t.rst", &source));
    let allocated = peak_bytes()?.saturating_sub(before);

    result?;
    // Copying the evidence into every piece would take PATH_LEN * CONTINUATIONS bytes, 610 MB.
    let limit = 250 * source.len();
    assert!(
        allocated <= limit,
        "{allocated} bytes of peak growth, limit {limit} for a {} byte document",
        source.len()
    );
    Ok(())
}
