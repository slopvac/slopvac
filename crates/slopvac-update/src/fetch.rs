//! HTTP behind a trait, so tests need no network.

use std::time::Duration;

use thiserror::Error;

use crate::messages::{UpdatePhrase, UpdateText};

/// Why a fetch failed.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The URL is not HTTPS.
    #[error("{}", UpdatePhrase::FetchNotHttps.as_static())]
    NotHttps,
    /// The response was larger than the limit.
    #[error("{}", too_large_text(*.0))]
    TooLarge(u64),
    /// Transport or status failure.
    #[error("{0}")]
    Network(String),
}

fn too_large_text(limit: u64) -> String {
    UpdateText::FetchTooLarge { limit }.to_string()
}

/// Hops the fetcher follows. GitHub release downloads need one, to the asset host.
const MAX_REDIRECTS: u32 = 5;

/// Whether a redirect may lead to `url`: HTTPS, no credentials, and one of the hosts that serve
/// GitHub release assets. These are `github.com`, `api.github.com` and the `githubusercontent.com`
/// hosts that releases redirect to (`release-assets.githubusercontent.com`,
/// `objects.githubusercontent.com`). A server that can redirect elsewhere could serve any bytes
/// from any host, so everything else is refused. Signatures still decide what gets installed.
#[must_use]
pub fn redirect_host_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    // Lowercase ASCII letters, digits, `-` and `.` only: no uppercase, percent-escapes, ports,
    // non-ASCII (IDN) labels or other tricks that a URL parser might read differently.
    let plain = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.';
    if host.is_empty() || !host.bytes().all(plain) {
        return false;
    }
    matches!(host, "github.com" | "api.github.com")
        || host
            .strip_suffix(".githubusercontent.com")
            .is_some_and(|label| !label.is_empty() && !label.contains(".."))
}

/// Fetch a URL into memory.
pub trait Fetcher {
    /// GET `url` and return the body, failing when it exceeds `limit` bytes.
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError>;
}

/// The real fetcher: HTTPS only, bounded time and size.
///
/// `ureq` applies `https_only` to every hop, so a redirect to plain HTTP fails too. Redirects are
/// followed here, not by `ureq`, so each hop can be checked against [`redirect_host_allowed`].
#[derive(Debug, Clone, Copy)]
pub struct UreqFetcher {
    timeout: Duration,
    allow_loopback_http: bool,
}

impl UreqFetcher {
    /// A fetcher whose whole exchange may take `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            allow_loopback_http: false,
        }
    }

    /// Test seam, compiled only into test builds: also allow plain HTTP to a loopback address, so a
    /// local listener can stand in for a server. Redirects are refused, because `ureq` cannot
    /// enforce HTTPS for the hops once `https_only` is off. Any other plain-HTTP URL is refused.
    #[cfg(test)]
    #[must_use]
    pub fn allowing_loopback_http(timeout: Duration) -> Self {
        Self {
            timeout,
            allow_loopback_http: true,
        }
    }

    fn scheme_allowed(self, url: &str) -> bool {
        if url.starts_with("https://") {
            return true;
        }
        let Some(rest) = url.strip_prefix("http://") else {
            return false;
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or_default();
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.split(']').next())
            .unwrap_or_else(|| host.split(':').next().unwrap_or_default());
        self.allow_loopback_http && matches!(host, "127.0.0.1" | "localhost" | "::1")
    }
}

fn network(phrase: UpdatePhrase) -> FetchError {
    FetchError::Network(phrase.as_static().to_owned())
}

/// What one request returned: the final response, or a redirect to follow.
enum Step<T> {
    Done(T),
    Redirect(String),
}

/// Run `request` on `start`, then on each redirect target, until it returns a response.
///
/// This is the whole redirect policy: at most [`MAX_REDIRECTS`] hops, and every target must pass
/// [`redirect_host_allowed`] before `request` is called on it. The first URL is the caller's.
fn follow<T>(
    start: &str,
    mut request: impl FnMut(&str) -> Result<Step<T>, FetchError>,
) -> Result<T, FetchError> {
    let mut url = start.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        match request(&url)? {
            Step::Done(response) => return Ok(response),
            Step::Redirect(location) => {
                if !redirect_host_allowed(&location) {
                    return Err(network(UpdatePhrase::FetchRedirectHost));
                }
                url = location;
            }
        }
    }
    Err(network(UpdatePhrase::FetchTooManyRedirects))
}

impl Fetcher for UreqFetcher {
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if !self.scheme_allowed(url) {
            return Err(FetchError::NotHttps);
        }
        // The deadline covers the whole exchange, redirects included.
        let started = std::time::Instant::now();
        let mut resp = follow(url, |url| {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(self.timeout.saturating_sub(started.elapsed())))
                .https_only(!self.allow_loopback_http)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .user_agent(concat!("slopvac/", env!("CARGO_PKG_VERSION")))
                .build()
                .into();
            let resp = agent.get(url).call().map_err(|e| match e {
                ureq::Error::RequireHttpsOnly(_) => FetchError::NotHttps,
                other => FetchError::Network(other.to_string()),
            })?;
            if !resp.status().is_redirection() {
                return Ok(Step::Done(resp));
            }
            if self.allow_loopback_http {
                return Err(network(UpdatePhrase::FetchRedirectsRefused));
            }
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| network(UpdatePhrase::FetchRedirectNoLocation))?;
            Ok(Step::Redirect(location.to_owned()))
        })?;
        let mut buf = Vec::new();
        let n = std::io::Read::read_to_end(
            &mut resp.body_mut().with_config().limit(limit + 1).reader(),
            &mut buf,
        )
        .map_err(|e| {
            let over = e
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ureq::Error>())
                .is_some_and(|inner| matches!(inner, ureq::Error::BodyExceedsLimit(_)));
            if over {
                FetchError::TooLarge(limit)
            } else {
                FetchError::Network(e.to_string())
            }
        })?;
        if n as u64 > limit {
            return Err(FetchError::TooLarge(limit));
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Instant;

    /// A one-connection server. It reads the request head, writes `reply`, then keeps the socket
    /// open until the returned sender is dropped (`hold`) or closes it at once.
    struct Server {
        url: String,
        listener: TcpListener,
        _release: Option<mpsc::Sender<()>>,
    }

    fn serve(reply: Vec<u8>, hold: bool) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let probe = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && sock.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = sock.write_all(&reply);
            let _ = sock.flush();
            if hold {
                // Keep the connection open until the test ends.
                let _ = rx.recv_timeout(Duration::from_secs(10));
            }
        });
        Server {
            url,
            listener: probe,
            _release: hold.then_some(tx),
        }
    }

    fn ok(body: &[u8]) -> Vec<u8> {
        let mut r =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn local() -> UreqFetcher {
        UreqFetcher::allowing_loopback_http(Duration::from_secs(5))
    }

    #[test]
    fn production_fetcher_refuses_plain_http_without_connecting() {
        let server = serve(ok(b"x"), false);
        server.listener.set_nonblocking(true).unwrap();
        let err = UreqFetcher::new(Duration::from_secs(5))
            .get(&server.url, 10)
            .unwrap_err();
        assert!(matches!(err, FetchError::NotHttps), "{err}");
        let accepted = server.listener.accept();
        assert!(
            matches!(&accepted, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the fetcher opened a connection"
        );
    }

    #[test]
    fn plain_http_is_refused_for_non_loopback_hosts_even_in_the_test_seam() {
        for url in [
            "http://example.invalid/a",
            "http://127.0.0.1.example.invalid/a",
            "ftp://127.0.0.1/a",
            "file:///etc/passwd",
        ] {
            let err = local().get(url, 10).unwrap_err();
            assert!(matches!(err, FetchError::NotHttps), "{url}: {err}");
        }
    }

    #[test]
    fn returns_a_body_that_fits_the_limit() {
        let server = serve(ok(b"hello"), false);
        assert_eq!(local().get(&server.url, 5).unwrap(), b"hello");
    }

    #[test]
    fn rejects_a_declared_length_over_the_limit() {
        let server = serve(ok(&[b'a'; 10]), false);
        let err = local().get(&server.url, 5).unwrap_err();
        assert!(matches!(err, FetchError::TooLarge(5)), "{err}");
    }

    #[test]
    fn rejects_an_undeclared_length_over_the_limit() {
        // No Content-Length: the body ends when the server closes, so only the cap stops it.
        let mut reply = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        reply.extend_from_slice(&[b'a'; 4096]);
        let server = serve(reply, false);
        let err = local().get(&server.url, 100).unwrap_err();
        assert!(matches!(err, FetchError::TooLarge(100)), "{err}");
    }

    #[test]
    fn the_test_seam_does_not_follow_redirects() {
        let target = serve(ok(b"reached"), false);
        target.listener.set_nonblocking(true).unwrap();
        let redirect = serve(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
                target.url
            )
            .into_bytes(),
            false,
        );
        let err = local().get(&redirect.url, 100).unwrap_err();
        assert!(matches!(err, FetchError::Network(_)), "{err}");
        let hit = target.listener.accept();
        assert!(
            matches!(&hit, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the redirect target was contacted: {hit:?}"
        );
    }

    #[test]
    fn redirects_may_lead_only_to_github_release_hosts() {
        for url in [
            "https://github.com/slopvac/slopvac/releases/download/v3.1.0/a.tar.gz",
            "https://api.github.com/repos/slopvac/slopvac/releases/latest",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1?x=y",
            "https://objects.githubusercontent.com/a",
            "https://github.com:443/a",
        ] {
            assert!(redirect_host_allowed(url), "{url}");
        }
    }

    #[test]
    fn redirects_to_other_hosts_schemes_or_credentials_are_refused() {
        for url in [
            "http://github.com/a",
            "https://evil.example/github.com",
            "https://github.com.evil.example/a",
            "https://evilgithub.com/a",
            "https://githubusercontent.com/a",
            "https://.githubusercontent.com/a",
            "https://evil.com/.githubusercontent.com",
            "https://user@github.com/a",
            "https://github.com@evil.example/a",
            "https://github.com:8443/a",
            "https://GitHub.com/a",
            "https://github.com%2f.evil.example/a",
            "https://github.com%2eevil.example/a",
            "https://%67ithub.com/a",
            "https://g\u{131}thub.com/a",
            "https://\u{e9}.githubusercontent.com/a",
            "https://xn--github-9d0c.com/a",
            "https://github.com./a",
            "https://github.com\\@evil.example/a",
            "/relative/path",
            "//evil.example/a",
            "",
        ] {
            assert!(!redirect_host_allowed(url), "{url}");
        }
    }

    const START: &str = "https://github.com/slopvac/slopvac/releases/download/v3.1.0/a";
    const ASSETS: &str = "https://release-assets.githubusercontent.com/a";

    type Asked = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// A transport that answers from a script and records every URL it was asked for.
    fn scripted(
        replies: Vec<(&str, Step<&'static str>)>,
    ) -> (
        Asked,
        impl FnMut(&str) -> Result<Step<&'static str>, FetchError>,
    ) {
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut replies: std::collections::HashMap<String, Step<&'static str>> = replies
            .into_iter()
            .map(|(url, step)| (url.to_owned(), step))
            .collect();
        let log = std::rc::Rc::clone(&asked);
        let request = move |url: &str| {
            log.borrow_mut().push(url.to_owned());
            replies
                .remove(url)
                .ok_or_else(|| FetchError::Network(format!("404 {url}")))
        };
        (asked, request)
    }

    #[test]
    fn an_allowed_redirect_returns_the_destination_response() {
        let (asked, request) = scripted(vec![
            (START, Step::Redirect(ASSETS.into())),
            (ASSETS, Step::Done("body")),
        ]);
        assert_eq!(follow(START, request).unwrap(), "body");
        assert_eq!(*asked.borrow(), [START, ASSETS]);
    }

    #[test]
    fn a_redirect_to_a_forbidden_host_is_never_requested() {
        let evil = "https://evil.example/a";
        let (asked, request) = scripted(vec![
            (START, Step::Redirect(evil.into())),
            (evil, Step::Done("evil body")),
        ]);
        let err = follow(START, request).unwrap_err();
        assert!(matches!(err, FetchError::Network(_)), "{err}");
        assert_eq!(*asked.borrow(), [START], "the forbidden host was contacted");
    }

    #[test]
    fn a_forbidden_host_in_the_middle_of_a_chain_stops_the_chain() {
        let evil = "http://github.com/a";
        let (asked, request) = scripted(vec![
            (START, Step::Redirect(ASSETS.into())),
            (ASSETS, Step::Redirect(evil.into())),
            (evil, Step::Done("evil body")),
        ]);
        assert!(follow(START, request).is_err());
        assert_eq!(*asked.borrow(), [START, ASSETS]);
    }

    /// `n` redirects between allowed hosts, then a response.
    fn chain(n: usize) -> Vec<(&'static str, Step<&'static str>)> {
        let urls: Vec<&'static str> = (0..=n)
            .map(|i| &*Box::leak(format!("https://github.com/hop/{i}").into_boxed_str()))
            .collect();
        let mut script: Vec<_> = (0..n)
            .map(|i| (urls[i], Step::Redirect(urls[i + 1].to_owned())))
            .collect();
        script.push((urls[n], Step::Done("end")));
        script
    }

    #[test]
    fn five_redirects_are_followed() {
        let script = chain(5);
        let start = script[0].0;
        let (_, request) = scripted(script);
        assert_eq!(follow(start, request).unwrap(), "end");
    }

    #[test]
    fn a_sixth_redirect_fails() {
        let script = chain(6);
        let start = script[0].0;
        let (asked, request) = scripted(script);
        let err = follow(start, request).unwrap_err();
        assert!(err.to_string().contains("too many redirects"), "{err}");
        assert_eq!(asked.borrow().len(), 6, "the sixth hop was requested");
    }

    #[test]
    fn error_statuses_fail() {
        let server = serve(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            false,
        );
        let err = local().get(&server.url, 10).unwrap_err();
        assert!(matches!(err, FetchError::Network(_)), "{err}");
    }

    #[test]
    fn a_stalled_body_ends_at_the_deadline() {
        // Promises 100 bytes, sends 7, then goes quiet with the socket open.
        let server = serve(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial".to_vec(),
            true,
        );
        let started = Instant::now();
        let err = UreqFetcher::allowing_loopback_http(Duration::from_millis(300))
            .get(&server.url, 1000)
            .unwrap_err();
        assert!(matches!(err, FetchError::Network(_)), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_server_that_never_answers_ends_at_the_deadline() {
        let server = serve(Vec::new(), true);
        let started = Instant::now();
        let err = UreqFetcher::allowing_loopback_http(Duration::from_millis(300))
            .get(&server.url, 1000)
            .unwrap_err();
        assert!(matches!(err, FetchError::Network(_)), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }
}
