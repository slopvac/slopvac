//! `slopvac claims` and `slopvac review --dry-run` through the built binary: stdout is the
//! published wire format, byte for byte. The golden files are the ones that the `slopvac-agents`
//! tests pin at the library boundary, so the binary and the library cannot drift apart. The
//! comparison is on raw bytes, so a changed line ending fails it.

use std::fs;
use std::path::Path;

use snapbox::cmd::{Command, cargo_bin};

const ORIGINAL: &str = include_str!("../../slopvac-agents/tests/fixtures/claims/original.md");
const REVISED: &str = include_str!("../../slopvac-agents/tests/fixtures/claims/revised.md");
const CLAIMS: &[u8] = include_bytes!("../../slopvac-agents/tests/fixtures/wire/claims.json");
const DRY_RUN_PLAN: &[u8] =
    include_bytes!("../../slopvac-agents/tests/fixtures/wire/plan-dry-run.json");

fn project() -> Result<tempfile::TempDir, std::io::Error> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("old.md"), ORIGINAL)?;
    fs::write(dir.path().join("new.md"), REVISED)?;
    Ok(dir)
}

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
}

/// Assert exit 0 and the exact stdout bytes. No normalization of line endings happens here.
fn assert_stdout(cmd: Command, golden: &[u8]) -> Result<(), std::io::Error> {
    let out = cmd.output()?;
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(golden),
        "the printed bytes differ from the golden"
    );
    assert_eq!(
        out.stdout, golden,
        "the printed bytes differ from the golden"
    );
    Ok(())
}

#[test]
fn claims_prints_the_pinned_json_report() -> Result<(), std::io::Error> {
    let dir = project()?;
    let cmd = slopvac(dir.path()).args(["claims", "new.md", "--original", "old.md"]);
    assert_stdout(cmd, CLAIMS)
}

#[test]
fn review_dry_run_prints_the_pinned_json_plan() -> Result<(), std::io::Error> {
    let dir = project()?;
    let cmd = slopvac(dir.path()).args([
        "review",
        "new.md",
        "--original",
        "old.md",
        "--dry-run",
        "--author-model",
        "gpt-5",
        "--reviewer-model",
        "gpt-5",
    ]);
    assert_stdout(cmd, DRY_RUN_PLAN)
}
