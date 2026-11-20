//! A program that never calls `launch::intercept` cannot start a plugin
//! through `Launcher::CurrentExe`. This test binary never calls it, so the
//! host must refuse before it spawns anything.
#![cfg(unix)]

mod common;

use common::{Allow, cfg, spec};
use slopvac_plugins::launch::Launcher;
use slopvac_plugins::{Plugin, PluginError, Teardown};

#[test]
fn a_program_without_the_launcher_entry_point_gets_a_typed_error_and_no_child() {
    let mut c = cfg();
    c.launcher = Launcher::CurrentExe;
    let err = Plugin::start(&spec("ok"), &c, &Allow).unwrap_err();
    assert_eq!(err.error, PluginError::LauncherNotInstalled);
    assert_eq!(err.error.slopvac_code(), "launcher-not-installed");
    assert_eq!(
        err.to_string(),
        "the running program did not install the slopvac plugin launcher, \
         so plugins and git plugin sources cannot start"
    );
    // Nothing started, so nothing was torn down and nothing is left undone.
    assert_eq!(err.teardown, Teardown::default());
}

#[test]
fn an_explicit_launcher_program_is_unaffected() {
    let p = Plugin::start(&spec("ok"), &cfg(), &Allow).expect("plugin starts");
    assert!(p.shutdown().clean());
}
