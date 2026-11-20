//! The text of each public error, as 3.0 builds published it before the message registry.

use std::path::PathBuf;

use rstest::rstest;
use slopvac_update::fetch::FetchError;
use slopvac_update::notice::RegistryError;
use slopvac_update::pipeline::UpdateError;
use slopvac_update::policy::PolicyError;
use slopvac_update::verify::VerifyError;

#[rstest]
#[case::not_https(FetchError::NotHttps.to_string(), "refusing a non-HTTPS URL")]
#[case::too_large(FetchError::TooLarge(10).to_string(), "response is larger than 10 bytes")]
#[case::network(FetchError::Network("boom".into()).to_string(), "boom")]
#[case::shape(RegistryError::Shape.to_string(), "unexpected registry response")]
#[case::fetch_passthrough(
    RegistryError::Fetch(FetchError::NotHttps).to_string(),
    "refusing a non-HTTPS URL"
)]
#[case::platform(
    UpdateError::UnsupportedPlatform.to_string(),
    "self update does not support this platform; download a release archive instead"
)]
#[case::download(
    UpdateError::Download { asset: "a.tar.gz".into(), source: FetchError::NotHttps }.to_string(),
    "download of a.tar.gz failed: refusing a non-HTTPS URL"
)]
#[case::not_newer(
    UpdateError::NotNewer { current: "3.1.0".into(), requested: "3.0.0".into() }.to_string(),
    "refusing to install 3.0.0: it is not newer than the running 3.1.0"
)]
#[case::prerelease(
    UpdateError::Prerelease("3.2.0-rc.1".into()).to_string(),
    "refusing to install 3.2.0-rc.1: it is a prerelease, and self update installs stable releases only"
)]
#[case::verify(
    UpdateError::Verify(VerifyError::Rejected).to_string(),
    "refusing to install: the signature does not verify"
)]
#[case::no_binary(
    UpdateError::NoBinary.to_string(),
    "the archive does not contain a `slopvac` binary"
)]
#[case::archive(UpdateError::Archive("x".into()).to_string(), "cannot read the archive: x")]
#[case::replace(
    UpdateError::Replace("x".into()).to_string(),
    "cannot replace the installed binary: x"
)]
#[case::missing_signature(
    VerifyError::MissingSignature.to_string(),
    "the release has no signature for this artifact"
)]
#[case::invalid(VerifyError::Invalid("x".into()).to_string(), "invalid signature or key: x")]
#[case::wrong_release(
    VerifyError::WrongRelease { expected: "e".into(), signed: "g".into() }.to_string(),
    "the signature is for a different release or file: expected `e`, signed `g`"
)]
#[case::not_regular(
    PolicyError::NotRegular(PathBuf::from("a/slopvac.toml")).to_string(),
    "a/slopvac.toml: not a regular file"
)]
#[case::policy_too_large(
    PolicyError::TooLarge(PathBuf::from("a/slopvac.toml")).to_string(),
    "a/slopvac.toml: larger than 1048576 bytes"
)]
#[case::unreadable(
    PolicyError::Unreadable { path: PathBuf::from("a/slopvac.toml"), reason: "denied".into() }
        .to_string(),
    "a/slopvac.toml: cannot read: denied"
)]
fn error_text_is_unchanged(#[case] actual: String, #[case] expected: &str) {
    assert_eq!(actual, expected);
}

#[test]
fn only_the_too_large_policy_error_has_a_code() {
    let path = PathBuf::from("slopvac.toml");
    assert_eq!(
        PolicyError::TooLarge(path.clone()).code(),
        Some("update-policy-too-large")
    );
    assert_eq!(PolicyError::NotRegular(path).code(), None);
}
