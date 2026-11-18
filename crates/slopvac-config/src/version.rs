//! The `schema_version` gate. It runs before validation so a 2.x file gets one clear error.

use crate::diag::{Code, Diagnostic};
use crate::tree::{Node, Value};

/// The schema version this build reads.
pub const SUPPORTED: i64 = 3;

/// Refuse a 2.x config, or a config from a newer slopvac, with one diagnostic.
///
/// A missing `schema_version` and a version below 3 both mean a 2.x config. Anything that is
/// not an integer is left to the schema, which reports the wrong type.
#[expect(
    clippy::result_large_err,
    reason = "the diagnostic is the public error type of this function and of its callers"
)]
pub fn check_version(root: &Node, text: &str) -> Result<(), Diagnostic> {
    let path = vec!["schema_version".to_owned()];
    let Some(entry) = root.entry("schema_version") else {
        return Err(Diagnostic {
            code: Code::LegacyVersion,
            message: "this file has no `schema_version`, so it is a 2.x config".into(),
            label: "slopvac 3 reads only version 3 files".into(),
            span: first_line(text),
            help: Some("run `slopvac migrate` to convert it".into()),
            path,
            data: None,
        });
    };
    match entry.node.value {
        Value::Int(n) if n < SUPPORTED => Err(Diagnostic {
            code: Code::LegacyVersion,
            message: format!("`schema_version = {n}` marks a 2.x config"),
            label: "slopvac 3 reads only version 3 files".into(),
            span: entry.node.span.clone(),
            help: Some("run `slopvac migrate` to convert it".into()),
            path,
            data: None,
        }),
        Value::Int(n) if n > SUPPORTED => Err(Diagnostic {
            code: Code::UnsupportedVersion,
            message: format!("`schema_version = {n}` is newer than this slopvac reads"),
            label: format!("this build reads version {SUPPORTED}"),
            span: entry.node.span.clone(),
            help: Some("upgrade slopvac".into()),
            path,
            data: None,
        }),
        _ => Ok(()),
    }
}

fn first_line(text: &str) -> std::ops::Range<usize> {
    0..text.find('\n').unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::parse_toml;

    fn gate(text: &str) -> Option<Code> {
        check_version(&parse_toml(text).unwrap(), text)
            .err()
            .map(|d| d.code)
    }

    #[test]
    fn version_three_passes() {
        assert_eq!(gate("schema_version = 3\n"), None);
    }

    #[test]
    fn missing_and_old_versions_are_legacy() {
        assert_eq!(gate("profile = \"strict\"\n"), Some(Code::LegacyVersion));
        assert_eq!(gate("schema_version = 2\n"), Some(Code::LegacyVersion));
    }

    #[test]
    fn a_newer_version_asks_for_an_upgrade() {
        assert_eq!(gate("schema_version = 4\n"), Some(Code::UnsupportedVersion));
    }

    #[test]
    fn a_non_integer_version_is_left_to_the_schema() {
        assert_eq!(gate("schema_version = \"3\"\n"), None);
    }

    #[test]
    fn legacy_error_points_at_migrate() {
        let text = "mode = \"prose\"\n";
        let d = check_version(&parse_toml(text).unwrap(), text).unwrap_err();
        assert_eq!(
            d.help.as_deref(),
            Some("run `slopvac migrate` to convert it")
        );
        assert_eq!(&text[d.span], "mode = \"prose\"");
    }
}
