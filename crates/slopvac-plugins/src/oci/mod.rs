//! A small OCI distribution client over blocking `ureq`, no async runtime.
//!
//! Scope: pull and push of artifacts, referrers with the tag-schema fallback and
//! tag listing, against GHCR and any compliant registry. Limits and behaviours
//! follow the threat model (`docs/contract/threat-model.md`, section 6.4):
//!
//! - manifests and indexes are capped at 4 MiB, blobs at the descriptor size and
//!   256 MiB, and every blob is hashed while it streams;
//! - redirects are followed by hand, at most 5, over HTTPS only (loopback may use
//!   HTTP), and `Authorization` is dropped when the host changes;
//! - a token `realm` must be HTTPS on the registry's host or a listed token host;
//! - TLS uses rustls with the platform verifier and has no switch to turn it off.

mod auth;
mod client;
mod http;
mod reference;
mod types;

pub use auth::{
    Anonymous, Challenge, Credential, CredentialSource, DefaultCredentials, EnvCredentials,
};
pub use client::{Client, Fetched};
pub use reference::{Reference, Target};
pub use types::{
    Descriptor, Digest, Index, MAX_BLOB_BYTES, MAX_MANIFEST_BYTES, MEDIA_EMPTY, MEDIA_INDEX,
    MEDIA_MANIFEST, Manifest,
};

use thiserror::Error;

/// Redirects followed per request.
pub const MAX_REDIRECTS: usize = 5;

/// Everything that can go wrong in the OCI client. URLs in messages carry no
/// credentials or query strings.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OciError {
    /// A reference, digest or descriptor is malformed.
    #[error("invalid OCI input: {0}")]
    Invalid(String),
    /// The transport failed.
    #[error("network error for {url}: {message}")]
    Network {
        /// Redacted URL.
        url: String,
        /// What failed.
        message: String,
    },
    /// The URL would send data over plain HTTP or to a host the rules forbid.
    #[error("refused URL {0}: HTTPS is required outside loopback")]
    InsecureUrl(String),
    /// A redirect broke a rule.
    #[error("bad redirect from {url}: {why}")]
    BadRedirect {
        /// Redacted URL.
        url: String,
        /// The rule.
        why: String,
    },
    /// The token realm is not on the registry's host.
    #[error("token realm {0} is not on the registry host; credentials were not sent")]
    RealmRejected(String),
    /// Credentials could not be read.
    #[error("credential error: {0}")]
    Credential(String),
    /// The registry answered 401 or 403. A private or missing package looks like this on GHCR.
    #[error("access denied or not found at {0}")]
    Denied(String),
    /// The registry has no such object.
    #[error("not found: {0}")]
    NotFound(String),
    /// Another status than expected.
    #[error("unexpected status {status} from {url}")]
    Status {
        /// HTTP status.
        status: u16,
        /// Redacted URL.
        url: String,
    },
    /// A body is larger than the limit.
    #[error("{what} exceeds the limit of {limit} bytes")]
    TooLarge {
        /// What was read.
        what: String,
        /// The limit.
        limit: u64,
    },
    /// A blob or manifest does not hash to its digest.
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch {
        /// From the descriptor or reference.
        expected: String,
        /// Computed.
        actual: String,
    },
    /// A blob has another length than its descriptor says.
    #[error("size mismatch: descriptor says {expected} bytes, received {actual}")]
    SizeMismatch {
        /// From the descriptor.
        expected: u64,
        /// Received.
        actual: u64,
    },
    /// A manifest media type the client does not accept.
    #[error("unsupported media type `{0}`")]
    UnsupportedMediaType(String),
}

/// Strip userinfo, query and fragment from a URL for messages.
pub(crate) fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let rest = match rest.split_once('/') {
        Some((auth, path)) => format!("{}/{path}", auth.rsplit('@').next().unwrap_or(auth)),
        None => rest.rsplit('@').next().unwrap_or(rest).to_owned(),
    };
    if scheme.is_empty() {
        rest
    } else {
        format!("{scheme}://{rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn redaction_strips_userinfo_query_and_fragment() {
        assert_eq!(
            redact_url("https://user:pw@ghcr.io/v2/a/b?token=abc#x"),
            "https://ghcr.io/v2/a/b"
        );
        assert_eq!(redact_url("https://u:p@host"), "https://host");
        assert_eq!(redact_url("https://ghcr.io/token"), "https://ghcr.io/token");
    }
}
