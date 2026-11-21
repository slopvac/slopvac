//! File URIs and absolute paths, converted both ways.
//!
//! A client names a file as `file:///…` with the bytes that RFC 3986 does not allow in a path
//! written as `%XX`. The server needs the path to find the file, and the URI to publish
//! diagnostics on it. A Unix path is a sequence of bytes and not always text, so both
//! directions work on bytes: a path that is not UTF-8 is kept as it is, and the caller that
//! needs text decides what to do with it.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use fluent_uri::Uri;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};

/// The bytes a URI path writes as `%XX`: all but the unreserved characters, `/`, `:` and `@`.
const ESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/')
    .remove(b':')
    .remove(b'@');

/// The `file:` URI of the absolute path `path`.
pub(crate) fn file_uri(path: &Path) -> String {
    format!(
        "file://{}",
        percent_encode(path.as_os_str().as_bytes(), ESCAPED)
    )
}

/// The path a `file:` URI names, with every `%XX` decoded, `%2F` included. `None` for a URI
/// that is not a `file:` URI of this machine, or does not parse.
pub(crate) fn file_path(uri: &str) -> Option<PathBuf> {
    let parsed = Uri::parse(uri).ok()?;
    if !parsed.scheme()?.eq_lowercase("file") {
        return None;
    }
    if let Some(authority) = parsed.authority()
        && !matches!(authority.as_str(), "" | "localhost")
    {
        return None;
    }
    let bytes = parsed.path().as_estr().decode().into_bytes().into_owned();
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case::plain("/work/slopvac.toml", "file:///work/slopvac.toml")]
    #[case::space("/work dir/a.toml", "file:///work%20dir/a.toml")]
    #[case::percent("/100%/a.toml", "file:///100%25/a.toml")]
    #[case::query_and_fragment("/a?b#c.toml", "file:///a%3Fb%23c.toml")]
    #[case::brackets("/a[1]/b.toml", "file:///a%5B1%5D/b.toml")]
    #[case::non_ascii("/é.toml", "file:///%C3%A9.toml")]
    fn a_path_is_encoded_and_decodes_back(#[case] path: &str, #[case] uri: &str) {
        assert_eq!(file_uri(Path::new(path)), uri);
        assert_eq!(file_path(uri), Some(PathBuf::from(path)));
    }

    #[test]
    fn an_encoded_slash_decodes_to_a_slash() {
        assert_eq!(
            file_path("file:///work%2Fsub/a.toml"),
            Some(PathBuf::from("/work/sub/a.toml"))
        );
    }

    #[test]
    fn a_path_that_is_not_utf8_keeps_its_bytes_in_both_directions() {
        let path = PathBuf::from(OsString::from_vec(b"/work/a\xFF.toml".to_vec()));
        let uri = file_uri(&path);
        assert_eq!(uri, "file:///work/a%FF.toml");
        assert_eq!(file_path(&uri), Some(path));
    }

    #[rstest::rstest]
    #[case::other_scheme("untitled:Untitled-1")]
    #[case::other_host("file://server/share/a.toml")]
    #[case::bad_escape("file:///a%zz.toml")]
    fn a_uri_that_is_not_a_local_file_has_no_path(#[case] uri: &str) {
        assert_eq!(file_path(uri), None);
    }

    #[test]
    fn the_local_host_names_this_machine() {
        assert_eq!(
            file_path("file://localhost/work/a.toml"),
            Some(PathBuf::from("/work/a.toml"))
        );
    }
}
