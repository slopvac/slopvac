//! Marker scanning does bounded work through setup, independent of runner load.
//! This binary has one test, so other setup calls cannot affect its process-wide counter.

use std::fmt::Write as _;
use std::fs;

use slopvac_agents::content::BlockKind;
use slopvac_agents::marker_scan_steps;
use slopvac_agents::setup::{Options, execute};

#[test]
fn marker_scan_stays_linear_on_many_code_blocks() {
    for blocks in [1_000, 2_000, 4_000] {
        let project = tempfile::tempdir().unwrap();
        let mut text = String::new();
        for i in 0..blocks {
            write!(text, "```\ncode {i}\n```\n\nline {i}\n\n").unwrap();
        }
        for i in 0..3 * blocks {
            writeln!(text, "filler {i}").unwrap();
        }
        let file = project.path().join("CLAUDE.md");
        fs::write(&file, &text).unwrap();
        let options = Options {
            harness: Some("claude".into()),
            list: false,
            remove: false,
            check: false,
            root: project.path().into(),
            block: BlockKind::Standard,
            no_skill: false,
            agent: true,
            reviewer_model: None,
        };
        let before = marker_scan_steps();
        execute(&options).unwrap();
        execute(&Options {
            remove: true,
            ..options
        })
        .unwrap();
        let steps = marker_scan_steps() - before;
        // Two scans may each visit every line and fence a fixed number of times.
        // Allow 128 extra lines for the installed steering block and its separator.
        let limit = 8 * (text.lines().count() + blocks + 128);
        assert!(
            0 < steps && steps <= limit,
            "{blocks} code blocks: {steps} marker-scan steps, limit {limit}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), text);
    }
}
