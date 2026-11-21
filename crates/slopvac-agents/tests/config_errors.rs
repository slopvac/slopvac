//! Config errors from `prime` keep their values out of the error chain.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::fs;
use std::path::Path;

use slopvac_agents::AgentsError;
use slopvac_agents::error::ConfigFailure;
use slopvac_agents::prime;

fn prime_out(root: &Path, args: &[&str]) -> Result<String, AgentsError> {
    let words = std::iter::once("prime").chain(args.iter().copied());
    let m = prime::command().try_get_matches_from(words).unwrap();
    prime::run(&m, &clap::Command::new("slopvac"), root)
}

/// Display and Debug of the error and of every `source()` level.
fn chain_texts(err: &AgentsError) -> Vec<String> {
    let mut out = vec![err.to_string(), format!("{err:?}")];
    let mut next = std::error::Error::source(err);
    while let Some(e) = next {
        out.push(e.to_string());
        out.push(format!("{e:?}"));
        next = e.source();
    }
    out
}

#[test]
fn invalid_setting_redacts_values_in_the_chain_but_keeps_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\n\n[guidance]\nlevel = \"zebraquokka\"\n",
    )
    .unwrap();
    let err = prime_out(dir.path(), &[]).unwrap_err();
    for text in chain_texts(&err) {
        assert!(!text.contains("zebraquokka"), "leaked in: {text}");
    }
    let shown = err.to_string();
    assert!(shown.contains("config-invalid-value"), "{shown}");
    assert!(shown.contains("/guidance/level"), "{shown}");
    assert!(shown.contains("slopvac.toml:4:"), "{shown}");
    let diagnostics = err.diagnostics().expect("config errors carry diagnostics");
    assert!(diagnostics.contains("zebraquokka"), "{diagnostics}");
}

#[test]
fn source_chain_reaches_the_config_failure_and_the_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = prime_out(dir.path(), &["--config", "nope/slopvac.toml"]).unwrap_err();
    let failure = std::error::Error::source(&err).expect("config errors have a source");
    assert!(failure.is::<ConfigFailure>());
    let io = failure
        .source()
        .expect("an unreadable file keeps its io error");
    assert!(io.is::<std::io::Error>());
}

#[test]
fn a_text_failure_keeps_its_code_and_its_source() {
    // The public Pattern API reaches the same `TextError` that claims propagates with `?`:
    // `(a+)+$` on 64 `a` plus `!` exhausts the PCRE2 match limit. No input to `compare`
    // reaches that failure: the built-in patterns are not exponential, and the quadratic
    // ones are bounded by time, not by the match limit.
    let pattern = slopvac_text::Pattern::new(r"(a+)+$", false).unwrap();
    let text_error = pattern
        .is_match(&format!("{}!", "a".repeat(64)))
        .expect_err("the match limit stops the search");
    let err = AgentsError::from(text_error);
    assert_eq!(err.code(), "claims-text-failed");
    let source = std::error::Error::source(&err).expect("the text error is the source");
    assert!(source.is::<slopvac_text::TextError>());
}

#[test]
fn an_unknown_key_that_is_a_private_term_never_reaches_the_chain() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\n\n[guidance]\nprojectcodename-orchid = \"x\"\n\n[rules.secretrule]\nenabled = 5\n",
    )
    .unwrap();
    let err = prime_out(dir.path(), &[]).unwrap_err();
    for text in chain_texts(&err) {
        for secret in ["projectcodename", "orchid", "secretrule"] {
            assert!(!text.contains(secret), "{secret} leaked in: {text}");
        }
    }
    let shown = err.to_string();
    assert!(shown.contains("/guidance/<key>"), "{shown}");
    let diagnostics = err.diagnostics().expect("diagnostics stay available");
    assert!(
        diagnostics.contains("projectcodename-orchid"),
        "{diagnostics}"
    );
}
