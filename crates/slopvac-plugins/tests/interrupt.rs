//! `kill_all_live` kills every plugin of the process, so this test runs in its own binary.
#![cfg(unix)]

mod common;

use common::*;
use serde_json::json;
use slopvac_plugins::{Plugin, kill_all_live};

#[test]
fn h18_an_interrupt_kills_every_live_plugin() {
    let dir = scratch("h18");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    kill_all_live();
    let gone = wait_gone(&pids);
    let r = p.tag("main", &docs(1));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived");
    assert!(r.is_err());
}
