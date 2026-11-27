//! The JSON that `slopvac claims` and `slopvac review` print, pinned byte for byte.
//!
//! The files under `fixtures/wire` are the published wire format. A change that alters one is
//! a contract change. Write them again with `SLOPVAC_BLESS_WIRE=1`, review the diff, and
//! never bless to make a failing test pass. The binary's own stdout is pinned against the same
//! files in the `slopvac` crate (`tests/agents_wire.rs`); the provider-backed plan stays here.

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_agents::claims;
use slopvac_agents::error::AgentsError;
use slopvac_agents::review::{
    self, ModelProvider, Payload, ProviderReply, ReportedItem, ReviewEnv,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ORIGINAL: &str = include_str!("fixtures/claims/original.md");
const REVISED: &str = include_str!("fixtures/claims/revised.md");

/// A provider that declares one model and reports another for the first two payload items.
struct Reporting;

impl ModelProvider for Reporting {
    fn name(&self) -> &'static str {
        "reporting"
    }
    fn model(&self) -> &'static str {
        "claude-x"
    }
    fn review(&self, _: &Payload) -> Result<ProviderReply, AgentsError> {
        Ok(ProviderReply::new(
            "reported",
            vec![ReportedItem::new(0, "gpt-5"), ReportedItem::new(1, "gpt-5")],
        ))
    }
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wire")
        .join(name)
}

fn assert_golden(name: &str, actual: &str) -> TestResult {
    let path = golden(name);
    if std::env::var_os("SLOPVAC_BLESS_WIRE").is_some() {
        fs::write(&path, actual)?;
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(actual, expected, "{name} changed on the wire");
    Ok(())
}

fn files(dir: &Path) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let (old, new) = (dir.join("old.md"), dir.join("new.md"));
    fs::write(&old, ORIGINAL)?;
    fs::write(&new, REVISED)?;
    Ok((old, new))
}

fn claims_json() -> Result<String, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let (old, new) = files(dir.path())?;
    let matches = claims::command().try_get_matches_from([
        "claims".to_owned(),
        new.display().to_string(),
        "--original".to_owned(),
        old.display().to_string(),
    ])?;
    Ok(claims::run(&matches)?)
}

fn plan_json(extra: &[&str], env: &ReviewEnv<'_>) -> Result<String, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let (old, new) = files(dir.path())?;
    let mut argv = vec![
        "review".to_owned(),
        new.display().to_string(),
        "--original".to_owned(),
        old.display().to_string(),
    ];
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    let matches = review::command().try_get_matches_from(argv)?;
    Ok(review::run(&matches, env)?)
}

#[test]
fn claims_report_bytes_are_stable() -> TestResult {
    assert_golden("claims.json", &claims_json()?)
}

#[test]
fn dry_run_plan_bytes_are_stable() -> TestResult {
    let plan = plan_json(
        &[
            "--dry-run",
            "--author-model",
            "gpt-5",
            "--reviewer-model",
            "gpt-5",
        ],
        &ReviewEnv::default(),
    )?;
    assert_golden("plan-dry-run.json", &plan)
}

#[test]
fn plan_with_a_reported_model_keeps_its_item_locations_byte_for_byte() -> TestResult {
    let env = ReviewEnv {
        provider: Some(&Reporting),
        ..ReviewEnv::default()
    };
    let plan = plan_json(&["--author-model", "gpt-5"], &env)?;
    assert_golden("plan-reported.json", &plan)
}
