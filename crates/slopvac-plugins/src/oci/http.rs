//! Redirect-aware HTTP on top of `ureq`.

use std::net::IpAddr;
use std::time::Duration;

use ureq::config::Config;
use ureq::http::{Request, Response, Uri};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body};

use super::{MAX_REDIRECTS, OciError, redact_url};

pub(crate) struct Transport {
    agent: Agent,
}

pub(crate) struct Req<'a> {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    /// Full `Authorization` header value, sent only to the first host.
    pub authorization: Option<String>,
    pub body: Option<&'a [u8]>,
}

pub(crate) fn host_of(url: &str) -> Option<(String, String)> {
    let uri: Uri = url.parse().ok()?;
    let scheme = uri.scheme_str()?.to_owned();
    let auth = uri.authority()?;
    Some((
        scheme,
        auth.as_str().rsplit('@').next()?.to_ascii_lowercase(),
    ))
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost" || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Host part of an authority (no port, no brackets removed for v6 literals).
pub(crate) fn bare_host(authority: &str) -> &str {
    if let Some(end) = authority.strip_prefix('[').and_then(|r| r.find(']')) {
        return &authority[..end + 2];
    }
    authority.rsplit_once(':').map_or(authority, |(h, _)| h)
}

/// Whether `url` may be requested: HTTPS, or HTTP on a loopback host.
pub(crate) fn url_allowed(url: &str) -> bool {
    match host_of(url) {
        Some((s, a)) if s == "https" => !a.is_empty(),
        Some((s, a)) if s == "http" => is_loopback_host(bare_host(&a)),
        _ => false,
    }
}

/// Resolve a `Location` value against the URL that returned it.
pub(crate) fn resolve(base: &str, location: &str) -> Option<String> {
    if location.contains("://") {
        return Some(location.to_owned());
    }
    let (scheme, rest) = base.split_once("://")?;
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    if let Some(r) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{authority}{location}"));
    }
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    if dir.is_empty() {
        Some(format!("{scheme}://{authority}/{location}"))
    } else {
        Some(format!("{scheme}://{authority}/{dir}/{location}"))
    }
}

impl Transport {
    pub(crate) fn new() -> Self {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = Config::builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(600)))
            .user_agent(concat!("slopvac/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub(crate) fn send(&self, req: &Req<'_>) -> Result<Response<Body>, OciError> {
        let mut url = req.url.clone();
        let mut auth = req.authorization.clone();
        let mut hops = 0;
        loop {
            if !url_allowed(&url) {
                return Err(OciError::InsecureUrl(redact_url(&url)));
            }
            let mut b = Request::builder().method(req.method).uri(&url);
            for (k, v) in &req.headers {
                b = b.header(*k, v);
            }
            if let Some(a) = &auth {
                b = b.header("Authorization", a);
            }
            let request = b
                .body(req.body.map(<[u8]>::to_vec).unwrap_or_default())
                .map_err(|e| OciError::Invalid(format!("request for {}: {e}", redact_url(&url))))?;
            let resp = self.agent.run(request).map_err(|e| OciError::Network {
                url: redact_url(&url),
                message: e.to_string(),
            })?;
            let status = resp.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return Ok(resp);
            }
            let bad = |why: &str| OciError::BadRedirect {
                url: redact_url(&url),
                why: why.to_owned(),
            };
            if !matches!(req.method, "GET" | "HEAD") {
                return Err(bad("redirect on a request that is not GET or HEAD"));
            }
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(bad("more than 5 redirects"));
            }
            let loc = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| bad("no Location header"))?;
            let next = resolve(&url, loc).ok_or_else(|| bad("unusable Location"))?;
            if !url_allowed(&next) {
                return Err(bad("target is not HTTPS"));
            }
            if host_of(&next).map(|h| h.1) != host_of(&url).map(|h| h.1) {
                auth = None;
            }
            url = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_loopback_http_is_allowed() {
        for ok in [
            "https://ghcr.io/v2/",
            "http://127.0.0.1:5000/v2/",
            "http://localhost/v2/",
            "http://[::1]:5000/x",
        ] {
            assert!(url_allowed(ok), "{ok}");
        }
        for bad in [
            "http://ghcr.io/v2/",
            "ftp://x/y",
            "file:///etc/passwd",
            "http://127.0.0.1.evil.com/",
            "/v2/",
            "https://",
        ] {
            assert!(!url_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn location_resolution() {
        let base = "https://ghcr.io/v2/a/b/blobs/sha256:x";
        assert_eq!(
            resolve(base, "https://cdn.example/x?sig=1").unwrap(),
            "https://cdn.example/x?sig=1"
        );
        assert_eq!(
            resolve(base, "/v2/other").unwrap(),
            "https://ghcr.io/v2/other"
        );
        assert_eq!(
            resolve(base, "//cdn.example/p").unwrap(),
            "https://cdn.example/p"
        );
        assert_eq!(
            resolve("https://ghcr.io/v2/a/", "next").unwrap(),
            "https://ghcr.io/v2/a/next"
        );
    }
}
