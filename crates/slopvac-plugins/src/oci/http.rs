//! Redirect-aware HTTP on top of `ureq`.

use std::net::IpAddr;
use std::time::Duration;

use ureq::config::Config;
use ureq::http::{Request, Response, Uri};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body};

use super::{MAX_REDIRECTS, OciError, redact_url};

/// Time limits for one exchange.
#[derive(Clone, Copy)]
pub(crate) struct Timeouts {
    /// Longest the request body may take to send.
    pub send_body: Duration,
    /// Longest a whole exchange may take, from connect to the last body byte.
    pub global: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            send_body: Duration::from_mins(30),
            global: Duration::from_hours(1),
        }
    }
}

pub(crate) struct Transport {
    agent: Agent,
    allow_loopback_http: bool,
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

/// Scheme, lowercase host and port of a URL, with the default port filled in.
/// `None` for a URL without a host or with userinfo.
pub(crate) type Origin = (String, String, u16);

pub(crate) fn origin_of(url: &str) -> Option<Origin> {
    let uri: Uri = url.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let auth = uri.authority()?;
    if auth.as_str().contains('@') {
        return None;
    }
    let port = match (uri.port_u16(), scheme.as_str()) {
        (Some(p), _) => p,
        (None, "https") => 443,
        (None, "http") => 80,
        _ => return None,
    };
    let host = uri.host().filter(|h| !h.is_empty())?.to_ascii_lowercase();
    Some((scheme, host, port))
}

/// Whether `url` may be requested: HTTPS, or, when `allow_loopback_http` is set
/// (test clients only), HTTP on a loopback host.
pub(crate) fn url_allowed(url: &str, allow_loopback_http: bool) -> bool {
    match host_of(url) {
        Some((s, a)) if s == "https" => !a.is_empty(),
        Some((s, a)) if s == "http" => allow_loopback_http && is_loopback_host(bare_host(&a)),
        _ => false,
    }
}

/// Remove `.` and `..` segments from an absolute path (RFC 3986 section 5.2.4).
fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut trailing = false;
    for seg in path.split('/').skip(1) {
        trailing = false;
        match seg {
            "." => trailing = true,
            ".." => {
                out.pop();
                trailing = true;
            }
            s => out.push(s),
        }
    }
    let mut r = String::from("/");
    r.push_str(&out.join("/"));
    if trailing && !r.ends_with('/') {
        r.push('/');
    }
    r
}

/// Resolve a URI reference against `base` (RFC 3986 section 5.2) for the forms
/// registries send: absolute, network-path, absolute-path, query-only and
/// relative-path.
pub(crate) fn resolve(base: &str, reference: &str) -> Option<String> {
    let (scheme, rest) = base.split_once("://")?;
    let rest = rest.split('#').next().unwrap_or(rest);
    let (authority, path_query) = rest
        .find(['/', '?'])
        .map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
    let (base_path, base_query) = match path_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_query, None),
    };
    let base_path = if base_path.is_empty() { "/" } else { base_path };

    // A fragment is never sent to a server, and a query added later would land
    // inside it.
    let reference = reference.split('#').next().unwrap_or(reference);
    let first_seg = reference.split(['/', '?']).next().unwrap_or("");
    if first_seg.contains(':') {
        return Some(reference.to_owned());
    }
    if let Some(r) = reference.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    if reference.is_empty() {
        let q = base_query.map_or(String::new(), |q| format!("?{q}"));
        return Some(format!("{scheme}://{authority}{base_path}{q}"));
    }
    if let Some(q) = reference.strip_prefix('?') {
        return Some(format!("{scheme}://{authority}{base_path}?{q}"));
    }
    let (ref_path, ref_query) = reference
        .split_once('?')
        .map_or((reference, String::new()), |(p, q)| (p, format!("?{q}")));
    let merged = if ref_path.starts_with('/') {
        ref_path.to_owned()
    } else {
        let dir = base_path.rsplit_once('/').map_or("", |(d, _)| d);
        format!("{dir}/{ref_path}")
    };
    Some(format!(
        "{scheme}://{authority}{}{ref_query}",
        remove_dot_segments(&merged)
    ))
}

impl Transport {
    /// `allow_loopback_http` is set only by the test-only client constructor;
    /// production transports refuse every `http://` URL, redirects included.
    pub(crate) fn new(allow_loopback_http: bool, timeouts: Timeouts) -> Self {
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
            .timeout_send_body(Some(timeouts.send_body))
            .timeout_global(Some(timeouts.global))
            .user_agent(concat!("slopvac/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            allow_loopback_http,
        }
    }

    /// Where a redirect from `url` to `location` may go. This is the only check a
    /// redirect target passes before the next request, so production transports
    /// stop here at any `http://` target, loopback included.
    fn redirect_target(&self, url: &str, location: &str) -> Result<String, &'static str> {
        let next = resolve(url, location).ok_or("unusable Location")?;
        if url_allowed(&next, self.allow_loopback_http) {
            Ok(next)
        } else {
            Err("target is not HTTPS")
        }
    }

    pub(crate) fn send(&self, req: &Req<'_>) -> Result<Response<Body>, OciError> {
        let mut url = req.url.clone();
        let mut auth = req.authorization.clone();
        let mut hops = 0;
        loop {
            if !url_allowed(&url, self.allow_loopback_http) {
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
            let next = self.redirect_target(&url, loc).map_err(bad)?;
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
    fn production_allows_only_https_and_tests_add_loopback_http() {
        for ok in ["https://ghcr.io/v2/", "https://127.0.0.1:5000/v2/"] {
            assert!(url_allowed(ok, false), "{ok}");
        }
        for loopback in [
            "http://127.0.0.1:5000/v2/",
            "http://localhost/v2/",
            "http://[::1]:5000/x",
        ] {
            assert!(!url_allowed(loopback, false), "{loopback}");
            assert!(url_allowed(loopback, true), "{loopback}");
        }
        for bad in [
            "http://ghcr.io/v2/",
            "ftp://x/y",
            "file:///etc/passwd",
            "http://127.0.0.1.evil.com/",
            "/v2/",
            "https://",
        ] {
            assert!(!url_allowed(bad, false), "{bad}");
            assert!(!url_allowed(bad, true), "{bad}");
        }
    }

    #[test]
    fn a_production_redirect_never_leaves_https_even_for_loopback() {
        let production = Transport::new(false, Timeouts::default());
        for location in [
            "http://127.0.0.1:5000/v2/",
            "http://localhost/x",
            "http://[::1]:5000/x",
            "//127.0.0.1:5000/x",
            "http://cdn.example/x",
        ] {
            // `//` resolves against the https base, so it stays HTTPS; the rest
            // name http:// and must be refused.
            let got = production.redirect_target("https://ghcr.io/v2/a/blobs/d", location);
            if location.starts_with("//") {
                assert_eq!(got.as_deref(), Ok("https://127.0.0.1:5000/x"));
            } else {
                assert_eq!(got, Err("target is not HTTPS"), "{location}");
            }
        }
        assert_eq!(
            production.redirect_target("https://ghcr.io/a", "https://cdn.example/b"),
            Ok("https://cdn.example/b".to_owned())
        );
    }

    #[test]
    fn the_test_transport_alone_follows_a_redirect_to_loopback_http() {
        let testing = Transport::new(true, Timeouts::default());
        assert_eq!(
            testing.redirect_target("http://127.0.0.1:1/a", "http://127.0.0.1:2/b"),
            Ok("http://127.0.0.1:2/b".to_owned())
        );
    }

    #[test]
    fn origins_compare_scheme_host_and_port() {
        let o = |u| origin_of(u);
        assert_eq!(o("https://GHCR.io/token"), o("https://ghcr.io:443/x"));
        assert_ne!(o("https://ghcr.io/token"), o("https://ghcr.io:8443/token"));
        assert_ne!(o("https://ghcr.io/token"), o("http://ghcr.io/token"));
        assert_eq!(o("https://u:p@ghcr.io/token"), None);
    }

    #[test]
    fn reference_resolution_follows_rfc_3986() {
        let base = "https://ghcr.io/v2/a/b/tags/list?n=1000";
        for (reference, want) in [
            ("https://cdn.example/x?sig=1", "https://cdn.example/x?sig=1"),
            ("//cdn.example/p", "https://cdn.example/p"),
            ("/v2/other?x=1", "https://ghcr.io/v2/other?x=1"),
            ("?last=1.0.0", "https://ghcr.io/v2/a/b/tags/list?last=1.0.0"),
            ("next?x=1", "https://ghcr.io/v2/a/b/tags/next?x=1"),
            ("../c", "https://ghcr.io/v2/a/b/c"),
            ("./", "https://ghcr.io/v2/a/b/tags/"),
            ("", "https://ghcr.io/v2/a/b/tags/list?n=1000"),
            (
                "?next=http://x/y",
                "https://ghcr.io/v2/a/b/tags/list?next=http://x/y",
            ),
        ] {
            assert_eq!(
                resolve(base, reference).as_deref(),
                Some(want),
                "{reference}"
            );
        }
        assert_eq!(
            resolve("https://ghcr.io", "next").as_deref(),
            Some("https://ghcr.io/next")
        );
    }
}
