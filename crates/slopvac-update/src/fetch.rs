//! HTTP behind a trait, so tests need no network.

use std::time::Duration;

use thiserror::Error;

/// Why a fetch failed.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The URL is not HTTPS.
    #[error("refusing a non-HTTPS URL")]
    NotHttps,
    /// The response was larger than the limit.
    #[error("response is larger than {0} bytes")]
    TooLarge(u64),
    /// Transport or status failure.
    #[error("{0}")]
    Network(String),
}

/// Fetch a URL into memory.
pub trait Fetcher {
    /// GET `url` and return the body, failing when it exceeds `limit` bytes.
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError>;
}

/// The real fetcher: HTTPS only, bounded time and size.
///
/// `ureq` applies `https_only` to every hop, so a redirect to plain HTTP fails too.
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

impl Fetcher for UreqFetcher {
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if !self.scheme_allowed(url) {
            return Err(FetchError::NotHttps);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .https_only(!self.allow_loopback_http)
            .max_redirects(if self.allow_loopback_http { 0 } else { 10 })
            .user_agent(concat!("slopvac/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "ureq::Error is non_exhaustive; every other failure is a network error"
        )]
        let mut resp = agent.get(url).call().map_err(|e| match e {
            ureq::Error::RequireHttpsOnly(_) => FetchError::NotHttps,
            other => FetchError::Network(other.to_string()),
        })?;
        if self.allow_loopback_http && resp.status().is_redirection() {
            return Err(FetchError::Network("redirects are refused".into()));
        }
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
        if !u64::try_from(n).is_ok_and(|n| n <= limit) {
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
            #[expect(
                clippy::let_underscore_must_use,
                reason = "test server: a client that hung up needs no reply"
            )]
            let _ = sock.write_all(&reply);
            #[expect(
                clippy::let_underscore_must_use,
                reason = "test server: a client that hung up needs no reply"
            )]
            let _ = sock.flush();
            if hold {
                // Keep the connection open until the test ends.
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "test server: the wait only keeps the connection open, so its result carries nothing"
                )]
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
