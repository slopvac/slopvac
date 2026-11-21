//! File URIs and absolute paths, converted both ways.
//!
//! A client names a file as `file:///…` with the characters that RFC 3986 does not allow in a
//! path written as `%XX`. The server needs the path to find the file, and the URI to publish
//! diagnostics on it. A Unix path is the only kind the server handles.

use std::fmt::Write as _;

const SCHEME: &str = "file://";

/// Whether `byte` stays as it is in the path of a URI: an unreserved character, `/`, or one
/// of the characters RFC 3986 lets a path segment hold.
const fn is_plain(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':' | b'@')
}

/// The `file:` URI of the absolute path `path`, with every other character encoded.
pub(crate) fn file_uri(path: &str) -> String {
    let mut uri = String::from(SCHEME);
    for byte in path.bytes() {
        if is_plain(byte) {
            uri.push(char::from(byte));
        } else {
            // Writing to a `String` cannot fail.
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

/// The path a `file:` URI names, with `%XX` sequences decoded. A text that is not a `file:`
/// URI is returned as it is, and so is a `%` that no two hex digits follow.
pub(crate) fn path_of(uri: &str) -> String {
    let rest = uri.strip_prefix(SCHEME).unwrap_or(uri);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let Some(byte) = decoded {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case::plain("/work/slopvac.toml", "file:///work/slopvac.toml")]
    #[case::space("/work dir/a.toml", "file:///work%20dir/a.toml")]
    #[case::percent("/100%/a.toml", "file:///100%25/a.toml")]
    #[case::query_and_fragment("/a?b#c.toml", "file:///a%3Fb%23c.toml")]
    #[case::non_ascii("/é.toml", "file:///%C3%A9.toml")]
    fn a_path_is_encoded_and_decodes_back(#[case] path: &str, #[case] uri: &str) {
        assert_eq!(file_uri(path), uri);
        assert_eq!(path_of(uri), path);
    }

    #[rstest::rstest]
    #[case::not_hex("file:///a%zzb", "/a%zzb")]
    #[case::cut_short("file:///a%4", "/a%4")]
    #[case::no_scheme("/a%20b", "/a b")]
    fn a_broken_escape_stays_as_written(#[case] uri: &str, #[case] path: &str) {
        assert_eq!(path_of(uri), path);
    }
}
