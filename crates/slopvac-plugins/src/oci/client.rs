//! The client: auth, manifests, blobs, referrers, tags and push.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use ureq::Body;
use ureq::http::Response;

use super::auth::{Challenge, CredentialSource, DefaultCredentials};
use super::http::{Req, Transport, bare_host, host_of, is_loopback_host, resolve, url_allowed};
use super::reference::{Reference, Target};
use super::types::{
    Descriptor, Digest, Index, MAX_BLOB_BYTES, MAX_MANIFEST_BYTES, MEDIA_INDEX, MEDIA_MANIFEST,
    Manifest,
};
use super::{OciError, redact_url};

const MANIFEST_ACCEPT: &str =
    "application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json";
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_TAG_PAGES: usize = 10;
const MAX_TAGS: usize = 10_000;

/// Token hosts that serve a registry from another host name.
const TOKEN_HOSTS: &[(&str, &str)] = &[
    ("registry-1.docker.io", "auth.docker.io"),
    ("docker.io", "auth.docker.io"),
];

struct CachedToken {
    value: String,
    expires: Instant,
}

#[derive(Deserialize)]
struct Tok {
    token: Option<String>,
    access_token: Option<String>,
    expires_in: Option<u64>,
}

/// A fetched manifest or index with its computed digest.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// Digest of `body`.
    pub digest: Digest,
    /// `application/vnd.oci.image.manifest.v1+json` or the index type.
    pub media_type: String,
    /// Raw bytes. Sign and hash these, never a re-encoding.
    pub body: Vec<u8>,
}

impl Fetched {
    /// Parse as an image manifest.
    ///
    /// # Errors
    /// [`OciError::UnsupportedMediaType`] for an index, [`OciError::Invalid`] for bad JSON.
    pub fn manifest(&self) -> Result<Manifest, OciError> {
        if self.media_type != MEDIA_MANIFEST {
            return Err(OciError::UnsupportedMediaType(self.media_type.clone()));
        }
        serde_json::from_slice(&self.body).map_err(|e| OciError::Invalid(format!("manifest: {e}")))
    }

    /// Parse as an image index.
    ///
    /// # Errors
    /// [`OciError::UnsupportedMediaType`] for a manifest, [`OciError::Invalid`] for bad JSON.
    pub fn index(&self) -> Result<Index, OciError> {
        if self.media_type != MEDIA_INDEX {
            return Err(OciError::UnsupportedMediaType(self.media_type.clone()));
        }
        serde_json::from_slice(&self.body).map_err(|e| OciError::Invalid(format!("index: {e}")))
    }
}

/// OCI registry client. One per run; it caches bearer tokens by scope.
pub struct Client {
    transport: Transport,
    creds: Box<dyn CredentialSource>,
    tokens: Mutex<HashMap<String, CachedToken>>,
}

impl Client {
    /// Client with credentials from the environment and the Docker config.
    #[must_use]
    pub fn new() -> Self {
        Self::with_credentials(Box::new(DefaultCredentials::from_env()))
    }

    /// Client with a given credential source.
    #[must_use]
    pub fn with_credentials(creds: Box<dyn CredentialSource>) -> Self {
        Self {
            transport: Transport::new(),
            creds,
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn base(r: &Reference) -> String {
        let scheme = if is_loopback_host(bare_host(&r.registry)) {
            "http"
        } else {
            "https"
        };
        format!("{scheme}://{}", r.registry)
    }

    fn cached(&self, scope: &str) -> Option<String> {
        let mut t = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match t.get(scope) {
            Some(c) if c.expires > Instant::now() => Some(c.value.clone()),
            Some(_) => {
                t.remove(scope);
                None
            }
            None => None,
        }
    }

    fn fetch_token(&self, r: &Reference, ch: &Challenge, scope: &str) -> Result<String, OciError> {
        let realm = ch
            .param("realm")
            .ok_or_else(|| OciError::Invalid("bearer challenge without realm".into()))?;
        let (scheme, authority) =
            host_of(realm).ok_or_else(|| OciError::RealmRejected(redact_url(realm)))?;
        let realm_host = bare_host(&authority).to_owned();
        let reg_host = r.host().to_ascii_lowercase();
        let listed = TOKEN_HOSTS
            .iter()
            .any(|(reg, tok)| *reg == reg_host && *tok == realm_host);
        let same = realm_host == reg_host;
        if !(same || listed)
            || !url_allowed(realm)
            || (scheme == "http" && !is_loopback_host(&realm_host))
        {
            return Err(OciError::RealmRejected(redact_url(realm)));
        }
        let mut url = realm.to_owned();
        url.push(if realm.contains('?') { '&' } else { '?' });
        if let Some(service) = ch.param("service") {
            let _ = write!(url, "service={}&", encode(service));
        }
        let _ = write!(url, "scope={}", encode(scope));
        let cred = self.creds.credential(&r.registry)?;
        let mut resp = self.transport.send(&Req {
            method: "GET",
            url: url.clone(),
            headers: vec![("Accept", "application/json".into())],
            authorization: cred.map(|c| c.basic_header()),
            body: None,
        })?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(if matches!(status, 401 | 403) {
                OciError::Denied(redact_url(&url))
            } else {
                OciError::Status {
                    status,
                    url: redact_url(&url),
                }
            });
        }
        let body = read_limited(resp.body_mut(), MAX_TOKEN_BYTES, "token response")?;
        let t: Tok = serde_json::from_slice(&body)
            .map_err(|_| OciError::Invalid("token response is not JSON".into()))?;
        let value = t
            .token
            .or(t.access_token)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| OciError::Denied(redact_url(&url)))?;
        let ttl = Duration::from_secs(t.expires_in.unwrap_or(60).clamp(15, 3600) - 10);
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                scope.to_owned(),
                CachedToken {
                    value: value.clone(),
                    expires: Instant::now() + ttl,
                },
            );
        Ok(value)
    }

    /// Send a request with lazy token acquisition and one retry after a 401.
    fn call(
        &self,
        r: &Reference,
        actions: &str,
        method: &'static str,
        url: &str,
        headers: &[(&'static str, String)],
        body: Option<&[u8]>,
    ) -> Result<Response<Body>, OciError> {
        let scope = format!("repository:{}:{actions}", r.repository);
        let make = |authorization: Option<String>| Req {
            method,
            url: url.to_owned(),
            headers: headers.to_vec(),
            authorization,
            body,
        };
        let first = self.cached(&scope).map(|t| format!("Bearer {t}"));
        let resp = self.transport.send(&make(first))?;
        if resp.status().as_u16() != 401 {
            return Ok(resp);
        }
        let challenge = resp
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .and_then(Challenge::parse);
        let Some(challenge) = challenge else {
            return Ok(resp);
        };
        let auth = match challenge.scheme.as_str() {
            "bearer" => format!("Bearer {}", self.fetch_token(r, &challenge, &scope)?),
            "basic" => match self.creds.credential(&r.registry)? {
                Some(c) => c.basic_header(),
                None => return Ok(resp),
            },
            _ => return Ok(resp),
        };
        self.transport.send(&make(Some(auth)))
    }

    fn deny_or_status(url: &str, status: u16) -> OciError {
        match status {
            401 | 403 => OciError::Denied(redact_url(url)),
            404 => OciError::NotFound(redact_url(url)),
            _ => OciError::Status {
                status,
                url: redact_url(url),
            },
        }
    }

    /// Fetch a manifest or index, verifying its digest when the reference is one.
    ///
    /// # Errors
    /// Any [`OciError`]: access denied, an oversized body, a digest mismatch, an unknown media type.
    pub fn fetch_manifest(&self, r: &Reference) -> Result<Fetched, OciError> {
        let url = format!(
            "{}/v2/{}/manifests/{}",
            Self::base(r),
            r.repository,
            r.target
        );
        let mut resp = self.call(
            r,
            "pull",
            "GET",
            &url,
            &[("Accept", MANIFEST_ACCEPT.into())],
            None,
        )?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(Self::deny_or_status(&url, status));
        }
        let header_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase());
        check_declared_length(&resp, MAX_MANIFEST_BYTES as u64, "manifest")?;
        let body = read_limited(resp.body_mut(), MAX_MANIFEST_BYTES, "manifest")?;
        let digest = Digest::of(&body);
        if let Target::Digest(want) = &r.target
            && *want != digest
        {
            return Err(OciError::DigestMismatch {
                expected: want.to_string(),
                actual: digest.to_string(),
            });
        }
        let media_type = match header_type.as_deref() {
            Some(t @ (MEDIA_MANIFEST | MEDIA_INDEX)) => t.to_owned(),
            other => {
                // Some registries send a generic type; the body then names its own.
                let declared = serde_json::from_slice::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("mediaType")
                            .and_then(|m| m.as_str().map(str::to_owned))
                    });
                match declared.as_deref() {
                    Some(t @ (MEDIA_MANIFEST | MEDIA_INDEX))
                        if other.is_none_or(|o| {
                            o.starts_with("application/json") || o == "text/plain"
                        }) =>
                    {
                        t.to_owned()
                    }
                    _ => {
                        return Err(OciError::UnsupportedMediaType(
                            other.unwrap_or("none").to_owned(),
                        ));
                    }
                }
            }
        };
        Ok(Fetched {
            digest,
            media_type,
            body,
        })
    }

    /// Stream a blob into `out`, checking size and digest as it goes.
    ///
    /// On `Err` the bytes already written must be discarded: the caller writes
    /// to a temporary file and keeps it only on `Ok`.
    ///
    /// # Errors
    /// [`OciError::TooLarge`] before any request when the descriptor exceeds 256 MiB,
    /// [`OciError::SizeMismatch`] and [`OciError::DigestMismatch`] after the stream.
    pub fn pull_blob(
        &self,
        r: &Reference,
        desc: &Descriptor,
        out: &mut dyn Write,
    ) -> Result<(), OciError> {
        if desc.size > MAX_BLOB_BYTES {
            return Err(OciError::TooLarge {
                what: "blob".into(),
                limit: MAX_BLOB_BYTES,
            });
        }
        let url = format!(
            "{}/v2/{}/blobs/{}",
            Self::base(r),
            r.repository,
            desc.digest
        );
        let mut resp = self.call(r, "pull", "GET", &url, &[], None)?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(Self::deny_or_status(&url, status));
        }
        check_declared_length(&resp, desc.size, "blob")?;
        let mut reader = resp.body_mut().with_config().limit(desc.size + 1).reader();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| OciError::Network {
                url: redact_url(&url),
                message: e.to_string(),
            })?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > desc.size {
                return Err(OciError::SizeMismatch {
                    expected: desc.size,
                    actual: total,
                });
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| OciError::Network {
                url: redact_url(&url),
                message: format!("write: {e}"),
            })?;
        }
        if total != desc.size {
            return Err(OciError::SizeMismatch {
                expected: desc.size,
                actual: total,
            });
        }
        let actual = Digest::from_hasher(hasher);
        if actual != desc.digest {
            return Err(OciError::DigestMismatch {
                expected: desc.digest.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

    /// Fetch a small blob into memory, up to `max` bytes (for example a signature statement).
    ///
    /// # Errors
    /// As [`Client::pull_blob`], plus [`OciError::TooLarge`] above `max`.
    pub fn fetch_blob_bytes(
        &self,
        r: &Reference,
        desc: &Descriptor,
        max: u64,
    ) -> Result<Vec<u8>, OciError> {
        if desc.size > max {
            return Err(OciError::TooLarge {
                what: "blob".into(),
                limit: max,
            });
        }
        let mut buf = Vec::new();
        self.pull_blob(r, desc, &mut buf)?;
        Ok(buf)
    }

    /// Manifests that refer to `subject`, through the referrers API or, when the
    /// registry lacks it (GHCR), the `sha256-<hex>` tag schema.
    ///
    /// # Errors
    /// Any [`OciError`] other than a missing API or tag, which give an empty list.
    pub fn referrers(
        &self,
        r: &Reference,
        subject: &Digest,
        artifact_type: Option<&str>,
    ) -> Result<Vec<Descriptor>, OciError> {
        let url = format!("{}/v2/{}/referrers/{subject}", Self::base(r), r.repository);
        let mut resp = self.call(
            r,
            "pull",
            "GET",
            &url,
            &[("Accept", MEDIA_INDEX.into())],
            None,
        )?;
        let status = resp.status().as_u16();
        let index: Option<Index> = if status == 200 {
            let body = read_limited(resp.body_mut(), MAX_MANIFEST_BYTES, "referrers index")?;
            Some(
                serde_json::from_slice(&body)
                    .map_err(|e| OciError::Invalid(format!("referrers index: {e}")))?,
            )
        } else if status == 404 {
            let tag = format!("sha256-{}", subject.hex());
            match self.fetch_manifest(&r.with_target(Target::Tag(tag))) {
                Ok(f) => Some(f.index()?),
                Err(OciError::NotFound(_)) => None,
                Err(e) => return Err(e),
            }
        } else {
            return Err(Self::deny_or_status(&url, status));
        };
        let mut found = index.map_or_else(Vec::new, |i| i.manifests);
        if let Some(want) = artifact_type {
            found.retain(|d| d.artifact_type.as_deref() == Some(want));
        }
        Ok(found)
    }

    /// List tags, following `Link` pagination up to 10 pages and 10,000 tags.
    ///
    /// # Errors
    /// Any [`OciError`].
    pub fn list_tags(&self, r: &Reference) -> Result<Vec<String>, OciError> {
        #[derive(Deserialize)]
        struct Tags {
            tags: Option<Vec<String>>,
        }
        let mut url = format!("{}/v2/{}/tags/list?n=1000", Self::base(r), r.repository);
        let mut all: Vec<String> = Vec::new();
        for _ in 0..MAX_TAG_PAGES {
            let mut resp = self.call(r, "pull", "GET", &url, &[], None)?;
            let status = resp.status().as_u16();
            if status != 200 {
                return Err(Self::deny_or_status(&url, status));
            }
            let next = resp
                .headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
                .and_then(|l| {
                    l.split_once('<')
                        .and_then(|(_, r)| r.split_once('>'))
                        .map(|(u, _)| u.to_owned())
                });
            let body = read_limited(resp.body_mut(), MAX_MANIFEST_BYTES, "tag list")?;
            let page: Tags = serde_json::from_slice(&body)
                .map_err(|e| OciError::Invalid(format!("tag list: {e}")))?;
            all.extend(page.tags.unwrap_or_default());
            if all.len() > MAX_TAGS {
                return Err(OciError::TooLarge {
                    what: "tag list".into(),
                    limit: MAX_TAGS as u64,
                });
            }
            let Some(next) = next.and_then(|n| resolve(&url, &n)) else {
                break;
            };
            if host_of(&next) != host_of(&url) {
                break;
            }
            url = next;
        }
        Ok(all)
    }

    /// Upload a blob in one request, skipping the upload when the registry has it.
    ///
    /// # Errors
    /// Any [`OciError`]; 401/403 on GHCR usually means the token lacks `write:packages`.
    pub fn push_blob(
        &self,
        r: &Reference,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<Descriptor, OciError> {
        let desc = Descriptor::for_bytes(media_type, bytes);
        if desc.size > MAX_BLOB_BYTES {
            return Err(OciError::TooLarge {
                what: "blob".into(),
                limit: MAX_BLOB_BYTES,
            });
        }
        let base = Self::base(r);
        let head = format!("{base}/v2/{}/blobs/{}", r.repository, desc.digest);
        let exists = self.call(r, "pull,push", "HEAD", &head, &[], None)?;
        if exists.status().as_u16() == 200 {
            return Ok(desc);
        }
        let start = format!("{base}/v2/{}/blobs/uploads/", r.repository);
        let resp = self.call(
            r,
            "pull,push",
            "POST",
            &start,
            &[("Content-Length", "0".into())],
            None,
        )?;
        let status = resp.status().as_u16();
        if status != 202 {
            return Err(Self::deny_or_status(&start, status));
        }
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .and_then(|l| resolve(&start, l))
            .ok_or_else(|| OciError::Invalid("upload response without Location".into()))?;
        if host_of(&location) != host_of(&start) {
            return Err(OciError::BadRedirect {
                url: redact_url(&start),
                why: "upload target is on another host".into(),
            });
        }
        let put = format!(
            "{location}{}digest={}",
            if location.contains('?') { '&' } else { '?' },
            desc.digest
        );
        let resp = self.call(
            r,
            "pull,push",
            "PUT",
            &put,
            &[("Content-Type", "application/octet-stream".into())],
            Some(bytes),
        )?;
        let status = resp.status().as_u16();
        if status != 201 {
            return Err(Self::deny_or_status(&put, status));
        }
        Ok(desc)
    }

    /// Put a manifest under the reference's tag or digest. Returns its digest.
    ///
    /// # Errors
    /// Any [`OciError`].
    pub fn push_manifest(
        &self,
        r: &Reference,
        media_type: &str,
        body: &[u8],
    ) -> Result<Digest, OciError> {
        if body.len() > MAX_MANIFEST_BYTES {
            return Err(OciError::TooLarge {
                what: "manifest".into(),
                limit: MAX_MANIFEST_BYTES as u64,
            });
        }
        let url = format!(
            "{}/v2/{}/manifests/{}",
            Self::base(r),
            r.repository,
            r.target
        );
        let resp = self.call(
            r,
            "pull,push",
            "PUT",
            &url,
            &[("Content-Type", media_type.into())],
            Some(body),
        )?;
        let status = resp.status().as_u16();
        if status != 201 && status != 200 {
            return Err(Self::deny_or_status(&url, status));
        }
        Ok(Digest::of(body))
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

fn check_declared_length(resp: &Response<Body>, limit: u64, what: &str) -> Result<(), OciError> {
    let declared = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    match declared {
        Some(n) if n > limit => Err(OciError::TooLarge {
            what: what.into(),
            limit,
        }),
        _ => Ok(()),
    }
}

fn read_limited(body: &mut Body, limit: usize, what: &str) -> Result<Vec<u8>, OciError> {
    let mut buf = Vec::new();
    body.with_config()
        .limit(limit as u64 + 1)
        .reader()
        .read_to_end(&mut buf)
        .map_err(|e| OciError::Network {
            url: what.to_owned(),
            message: e.to_string(),
        })?;
    if buf.len() > limit {
        return Err(OciError::TooLarge {
            what: what.into(),
            limit: limit as u64,
        });
    }
    Ok(buf)
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn query_encoding_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(
            encode("repository:a/b:pull,push"),
            "repository%3Aa%2Fb%3Apull%2Cpush"
        );
        assert_eq!(encode("ghcr.io"), "ghcr.io");
    }
}
