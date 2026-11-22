//! A tag whose evidence is long, and that is split into many pieces, holds the evidence
//! once, so the work to build the tags follows the size of the document.
//!
//! The document is an RST simple table. Its last column holds one sentence that says
//! "no" before a very long path, and then runs over many short continuation lines. The
//! middle column wraps beside it, so the sentence's tag is split into one piece for each
//! line. The test counts the pieces built and the bytes of evidence text that tags hold in
//! allocations of their own, with the `work-counters` feature. It counts no time and reads
//! no `/proc`. It runs where the fake interpreter script does, on Unix.
#![cfg(unix)]

mod common;

use serde_json::json;
use slopvac_context::{take_evidence_bytes, take_materialised_tags};
use slopvac_engine::Input;

const PATH_LEN: usize = 60_000;
const CONTINUATIONS: usize = 10_000;

fn source() -> String {
    let mut s = String::from("=  =  ====\n");
    s.push_str("A  m  It has no /");
    s.push_str(&"a".repeat(PATH_LEN));
    s.push('\n');
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

    let _ = (take_evidence_bytes(), take_materialised_tags());
    engine.lint(&Input::rst("t.rst", &source))?;
    let (evidence, pieces) = (take_evidence_bytes(), take_materialised_tags());

    // The sentence has one piece for each of its lines, so the trigger split the tag.
    assert!(
        pieces > CONTINUATIONS,
        "{pieces} tags built, but the sentence has {CONTINUATIONS} continuation lines"
    );
    // The requirement tagger fired: its evidence quotes the path, so it holds at least that.
    assert!(
        evidence >= PATH_LEN,
        "{evidence} bytes of evidence, but the path alone has {PATH_LEN}"
    );
    // The evidence is held once, plus a short line for each other tag. Copying it into
    // every piece would hold PATH_LEN * CONTINUATIONS bytes, 600 MB.
    let limit = 2 * (PATH_LEN + CONTINUATIONS);
    assert!(
        evidence <= limit,
        "{evidence} bytes of evidence, limit {limit} for a {PATH_LEN} byte path over {CONTINUATIONS} lines"
    );
    Ok(())
}
