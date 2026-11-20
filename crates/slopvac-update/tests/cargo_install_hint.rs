//! A cargo-style install is told to use the releases page, never a crate name that
//! crates.io could hand to someone else.

use std::sync::Arc;

use slopvac_update::command::{self, Runtime};
use slopvac_update::fetch::{FetchError, Fetcher};
use slopvac_update::install::InstallMethod;
use slopvac_update::notice::notice_for;

const RELEASES: &str = "https://github.com/slopvac/slopvac/releases";

struct Offline;
impl Fetcher for Offline {
    fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Network(format!("unexpected request to {url}")))
    }
}

#[test]
fn the_update_notice_points_cargo_installs_at_the_releases_page() {
    let n = notice_for("3.0.0", "3.1.0", InstallMethod::Cargo).expect("notice");
    assert_eq!(n.upgrade_command, None);
    assert_eq!(
        n.message,
        format!("slopvac 3.1.0 is available (you have 3.0.0). See {RELEASES}.")
    );
}

#[test]
fn self_update_on_a_cargo_install_points_at_the_releases_page() {
    let dir = tempfile::tempdir().unwrap();
    let cargo_home = dir.path().join("home/.cargo");
    std::fs::create_dir_all(cargo_home.join("bin")).unwrap();
    let exe = cargo_home.join("bin/slopvac");
    std::fs::write(&exe, b"old binary").unwrap();
    let home = cargo_home.display().to_string();
    let var = Box::new(move |k: &str| (k == "CARGO_HOME").then(|| home.clone()));
    let mut rt = Runtime::with_var(false, "3.0.0", None, var);
    rt.exe.clone_from(&exe);
    rt.fetcher = Arc::new(Offline);

    let matches = command::command().get_matches_from(["self", "update"]);
    let err = command::run(&matches, &rt).unwrap_err();

    assert_eq!(err.code, "self-update-unmanaged");
    assert_eq!(
        err.message,
        format!(
            "this install (cargo) is not managed by the slopvac installer; download a release from {RELEASES}"
        )
    );
    assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
}
