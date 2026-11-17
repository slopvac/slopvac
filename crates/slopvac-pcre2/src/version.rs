//! The minimum PCRE2 version check. Pure Rust apart from reading the version string.

use std::sync::OnceLock;

use crate::ffi;

/// Oldest supported PCRE2: 10.43 added bounded variable-length lookbehind, which the core
/// rules use (`prose-craft.future-tense`).
const MINIMUM: (u32, u32) = (10, 43);

/// The linked PCRE2 cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VersionError {
    /// A PCRE2 older than 10.43 is linked, for example a system `libpcre2`.
    #[error(
        "linked PCRE2 is {found}, but slopvac needs 10.43 or newer; build with \
         PCRE2_SYS_STATIC=1 to use the vendored library"
    )]
    TooOld {
        /// The version string PCRE2 reported.
        found: String,
    },
    /// The version string has an unknown shape.
    #[error("cannot read the linked PCRE2 version {found:?}")]
    Unreadable {
        /// The version string PCRE2 reported.
        found: String,
    },
}

/// The linked PCRE2's version string, such as `10.46 2025-08-27`.
#[must_use]
pub fn linked_version() -> String {
    ffi::version_string()
}

/// Fail unless the linked PCRE2 is at least 10.43. The result is computed once.
/// [`Regex::new`](crate::Regex::new) calls this, and the engine can call it at startup.
pub fn ensure_supported_version() -> Result<(), VersionError> {
    static CHECKED: OnceLock<Result<(), VersionError>> = OnceLock::new();
    CHECKED.get_or_init(|| check(&linked_version())).clone()
}

/// Check a version string of the form `MAJOR.MINOR[ date]` against [`MINIMUM`].
pub(crate) fn check(version: &str) -> Result<(), VersionError> {
    match parse(version) {
        Some(parsed) if parsed >= MINIMUM => Ok(()),
        Some(_) => Err(VersionError::TooOld {
            found: version.to_owned(),
        }),
        None => Err(VersionError::Unreadable {
            found: version.to_owned(),
        }),
    }
}

/// `"10.46 2025-08-27"` -> `(10, 46)`; `"10.44-RC1 ..."` -> `(10, 44)`.
fn parse(version: &str) -> Option<(u32, u32)> {
    let (major, rest) = version.split_once('.')?;
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    Some((major.parse().ok()?, rest[..digits].parse().ok()?))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("10.43 2024-02-16", true)]
    #[case("10.46 2025-08-27", true)]
    #[case("10.44-RC1 2024-06-07", true)]
    #[case("11.0 2030-01-01", true)]
    #[case("10.100", true)]
    #[case("10.42 2022-12-11", false)]
    #[case("10.9", false)]
    #[case("9.99", false)]
    fn versions_compare_numerically(#[case] version: &str, #[case] supported: bool) {
        assert_eq!(check(version).is_ok(), supported, "{version}");
    }

    #[rstest]
    #[case("")]
    #[case("10")]
    #[case("ten.forty")]
    #[case("10.")]
    fn unreadable_versions_are_rejected_not_accepted(#[case] version: &str) {
        assert!(matches!(
            check(version),
            Err(VersionError::Unreadable { .. })
        ));
    }

    #[test]
    fn the_too_old_message_names_the_remedy() {
        let message = check("10.42 2022-12-11").unwrap_err().to_string();
        assert!(message.contains("10.42 2022-12-11"), "{message}");
        assert!(message.contains("PCRE2_SYS_STATIC=1"), "{message}");
    }

    #[test]
    fn the_linked_library_meets_the_minimum() {
        assert_eq!(ensure_supported_version(), Ok(()), "{}", linked_version());
    }
}
