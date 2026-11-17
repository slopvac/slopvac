//! A small OCI distribution client over blocking `ureq`, no async runtime.
//!
//! Scope: pull and push of artifacts, referrers with the tag-schema fallback and
//! tag listing, against GHCR and any compliant registry. Limits and behaviours
//! follow the threat model (`docs/contract/threat-model.md`, section 6.4):
//!
//! - manifests and indexes are capped at 4 MiB, blobs at the descriptor size and
//!   256 MiB, and every blob is hashed while it streams;
//! - redirects are followed by hand, at most 5, over HTTPS only, and
//!   `Authorization` is dropped when the host changes. Plain HTTP exists only in
//!   the `#[doc(hidden)]` test constructor `Client::with_loopback_http_for_tests`;
//! - a token `realm` must be HTTPS on the registry's exact origin (scheme, host and
//!   port) or on a token service listed for that registry (Docker Hub's
//!   `auth.docker.io`), and cached tokens are keyed by registry origin and scope;
//! - TLS uses rustls with the platform verifier and has no switch to turn it off.

mod auth;
mod client;
mod http;
mod process;
mod reference;
mod types;

pub(crate) use auth::find_in_path;
pub use auth::{
    Anonymous, Challenge, Credential, CredentialSource, DefaultCredentials, EnvCredentials,
};
pub use client::{Client, Fetched};
pub(crate) use process::Contained;
pub(crate) use reference::valid_tag;
pub use reference::{Reference, Target};
pub use types::{
    Descriptor, Digest, Index, MAX_BLOB_BYTES, MAX_MANIFEST_BYTES, MEDIA_EMPTY, MEDIA_INDEX,
    MEDIA_MANIFEST, Manifest,
};

use slopvac_sanitize::line as esc;
use thiserror::Error;

/// Redirects followed per request.
pub const MAX_REDIRECTS: usize = 5;

/// Everything that can go wrong in the OCI client. URLs in messages carry no
/// credentials or query strings.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OciError {
    /// A reference, digest or descriptor is malformed.
    #[error("invalid OCI input: {}", esc(.0))]
    Invalid(String),
    /// The transport failed.
    #[error("network error for {}: {}", esc(.url), esc(.message))]
    Network {
        /// Redacted URL.
        url: String,
        /// What failed.
        message: String,
    },
    /// The URL would send data over plain HTTP or to a host the rules forbid.
    #[error("refused URL {}: HTTPS is required", esc(.0))]
    InsecureUrl(String),
    /// A redirect broke a rule.
    #[error("bad redirect from {}: {}", esc(.url), esc(.why))]
    BadRedirect {
        /// Redacted URL.
        url: String,
        /// The rule.
        why: String,
        /// Raw bytes read from the socket before the refusal, for callers that
        /// keep a shared byte budget. Zero when nothing was read.
        wire_bytes: u64,
    },
    /// The token realm is not on the registry's origin or a listed token service.
    #[error("token realm {} is not on the registry origin; credentials were not sent", esc(.0))]
    RealmRejected(String),
    /// Credentials could not be read.
    #[error("credential error: {}", esc(.0))]
    Credential(String),
    /// A credential helper exists only in a directory that is not trusted.
    /// `path` is already redacted for display.
    #[error(
        "credential helper `{}` was not found in a trusted directory; it exists only at {}. \
         Set SLOPVAC_OCI_USER and SLOPVAC_OCI_TOKEN (or GITHUB_TOKEN for ghcr.io) to sign in \
         without it; these take precedence over the Docker config",
        esc(.program),
        esc(.path)
    )]
    HelperUntrusted {
        /// The helper program name.
        program: String,
        /// Where it was found, project-relative or `<external>/<file name>`.
        path: String,
    },
    /// A credential helper was not run because the project directory is unknown,
    /// so slopvac cannot tell whether the helper lies inside it.
    #[error("credential helper `{}` was not run: the project directory is unknown", esc(.0))]
    HelperProjectUnknown(String),
    /// The registry answered 401 or 403. A private or missing package looks like this on GHCR.
    #[error("access denied or not found at {}", esc(.0))]
    Denied(String),
    /// The registry has no such object.
    #[error("not found: {}", esc(.0))]
    NotFound(String),
    /// Another status than expected.
    #[error("unexpected status {status} from {}", esc(.url))]
    Status {
        /// HTTP status.
        status: u16,
        /// Redacted URL.
        url: String,
    },
    /// A body is larger than the limit.
    #[error("{} exceeds the limit of {limit} bytes", esc(.what))]
    TooLarge {
        /// What was read.
        what: String,
        /// The limit.
        limit: u64,
    },
    /// A blob or manifest does not hash to its digest.
    #[error("digest mismatch: expected {}, got {}", esc(.expected), esc(.actual))]
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
    #[error("unsupported media type `{}`", esc(.0))]
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

impl OciError {
    /// The stable error code of the contract (policies.md, section 3.5), where
    /// one exists.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::HelperUntrusted { .. } => Some("credential-helper-untrusted"),
            Self::HelperProjectUnknown(_) => Some("credential-helper-project-unknown"),
            _ => None,
        }
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
