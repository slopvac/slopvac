//! A registry that follows a small chunked body with an endless trailer, or
//! wraps a small body in tiny TLS records, is cut off, whatever the fetch
//! (threat model section 6.4, resource exhaustion). The cap counts raw bytes
//! read from the socket, below TLS and below a proxy.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use slopvac_plugins::oci::{Anonymous, Client, Descriptor, OciError, Reference};

/// Trailer bytes the scripted registry tries to send: far past every cap.
const FLOOD: usize = 96 << 20;

const ROOT: &[u8] = include_bytes!("fixtures/tls/root.der");
const SERVER_CERT: &[u8] = include_bytes!("fixtures/tls/server.der");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/tls/server.key.der");

/// What the scripted registry answers for one request path.
struct Route {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// Send the body as one chunk and follow it with trailer lines until
    /// `FLOOD` bytes are out or the client hangs up.
    flood: bool,
}

impl Route {
    fn ok(body: &[u8]) -> Self {
        Self {
            status: "200 OK",
            headers: vec![],
            body: body.to_vec(),
            flood: false,
        }
    }

    fn flooding(body: &[u8]) -> Self {
        Self {
            flood: true,
            ..Self::ok(body)
        }
    }
}

struct Registry {
    port: u16,
    /// Trailer bytes written, sent once per flooded connection when it ends.
    written: mpsc::Receiver<usize>,
}

fn registry(route: impl Fn(u16, &str) -> Route + Send + Sync + 'static) -> Registry {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, written) = mpsc::channel();
    let route = Arc::new(route);
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (tx, route) = (tx.clone(), Arc::clone(&route));
            thread::spawn(move || {
                let Some(path) = read_head(&mut &conn) else {
                    return;
                };
                let route = route(port, &path);
                let flood = route.flood;
                let n = respond(conn, &route);
                if flood {
                    let _ = tx.send(n);
                }
            });
        }
    });
    Registry { port, written }
}

/// The request path, after the head has been read.
fn read_head(conn: &mut impl Read) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if conn.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let line = String::from_utf8_lossy(&head);
    line.split(' ').nth(1).map(str::to_owned)
}

/// Write the response; the number of trailer bytes that went out.
fn respond(mut conn: TcpStream, route: &Route) -> usize {
    let mut head = format!("HTTP/1.1 {}\r\n", route.status);
    for (k, v) in &route.headers {
        let _ = write!(head, "{k}: {v}\r\n");
    }
    if !route.flood {
        let _ = write!(head, "Content-Length: {}\r\n\r\n", route.body.len());
        let _ = conn.write_all(head.as_bytes());
        let _ = conn.write_all(&route.body);
        return 0;
    }
    let _ = write!(
        head,
        "Transfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        route.body.len()
    );
    let mut ok = conn.write_all(head.as_bytes()).is_ok()
        && conn.write_all(&route.body).is_ok()
        && conn.write_all(b"\r\n0\r\n").is_ok();
    let line = format!("X-Flood: {}\r\n", "a".repeat(1000));
    let batch = line.repeat(64);
    let mut written = 0;
    while ok && written < FLOOD {
        ok = conn.write_all(batch.as_bytes()).is_ok();
        if ok {
            written += batch.len();
        }
    }
    written
}

impl Registry {
    fn reference(&self) -> Reference {
        Reference::parse(&format!("127.0.0.1:{}/a/b:1", self.port)).unwrap()
    }

    /// The server stopped well before the flood ended: the client hung up. The
    /// count includes what the kernel buffered, so the bound is loose.
    fn assert_cut_off(&self) {
        let written = self
            .written
            .recv_timeout(Duration::from_secs(30))
            .expect("the registry never finished");
        assert!(
            written < FLOOD / 2,
            "the registry wrote {written} trailer bytes before the client hung up"
        );
    }
}

fn client() -> Client {
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
}

fn assert_refused<T: std::fmt::Debug>(out: &Result<T, OciError>) {
    assert!(
        matches!(out, Err(OciError::TooLarge { .. })),
        "expected TooLarge, got {out:?}"
    );
}

const MANIFEST: &[u8] = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.empty.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":2},"layers":[]}"#;

#[test]
fn a_manifest_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|_, _| Route::flooding(MANIFEST));
    assert_refused(&client().fetch_manifest(&reg.reference()));
    reg.assert_cut_off();
}

#[test]
fn a_blob_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|_, _| Route::flooding(b"hello"));
    let desc = Descriptor::for_bytes("x", b"hello");
    assert_refused(&client().pull_blob(&reg.reference(), &desc, &mut Vec::new()));
    reg.assert_cut_off();
}

#[test]
fn a_revocation_sized_blob_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|_, _| Route::flooding(b"{}"));
    let desc = Descriptor::for_bytes("x", b"{}");
    assert_refused(&client().fetch_blob_bytes(&reg.reference(), &desc, 64 << 10));
    reg.assert_cut_off();
}

#[test]
fn a_tag_list_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|_, _| Route::flooding(br#"{"name":"a/b","tags":["t1"]}"#));
    assert_refused(&client().list_tags(&reg.reference()));
    reg.assert_cut_off();
}

#[test]
fn a_token_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|port, path| {
        if path.starts_with("/token") {
            return Route::flooding(br#"{"token":"t"}"#);
        }
        Route {
            status: "401 Unauthorized",
            headers: vec![(
                "WWW-Authenticate",
                format!("Bearer realm=\"http://127.0.0.1:{port}/token\",service=\"r\""),
            )],
            ..Route::ok(b"")
        }
    });
    assert_refused(&client().fetch_manifest(&reg.reference()));
    reg.assert_cut_off();
}

#[test]
fn a_download_followed_by_a_trailer_flood_is_refused() {
    let reg = registry(|_, _| Route::flooding(b"abc"));
    let url = format!("https://127.0.0.1:{}/archive", reg.port);
    assert_refused(&client().download(&url, 1000, &mut Vec::new()));
    reg.assert_cut_off();
}

#[test]
fn a_chunked_blob_with_a_short_trailer_still_downloads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            if read_head(&mut &conn).is_none() {
                continue;
            }
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX-Sum: 1\r\n\r\n",
            );
        }
    });
    let desc = Descriptor::for_bytes("x", b"hello");
    let r = Reference::parse(&format!("127.0.0.1:{port}/a/b:1")).unwrap();
    let mut out = Vec::new();
    client().pull_blob(&r, &desc, &mut out).unwrap();
    assert_eq!(out, b"hello");
}

#[test]
fn a_plain_download_still_works() {
    let reg = registry(|_, _| Route::ok(b"archive"));
    let url = format!("https://127.0.0.1:{}/archive", reg.port);
    let mut out = Vec::new();
    client().download(&url, 1000, &mut out).unwrap();
    assert_eq!(out, b"archive");
}

// ---- TLS and proxies ----

fn tls_config() -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(SERVER_CERT)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SERVER_KEY)),
        )
        .unwrap();
    Arc::new(config)
}

/// How a TLS registry cuts its body into records.
#[derive(Clone, Copy)]
enum Records {
    /// Bytes of body per TLS record.
    Of(usize),
}

/// A TLS registry serving `body` to every request, `record` body bytes per TLS
/// record. Its plaintext is exactly the response, so only counting the socket
/// shows the cost of the records.
fn tls_registry(body: Vec<u8>, record: Records) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = tls_config();
    let body = Arc::new(body);
    thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (config, body) = (Arc::clone(&config), Arc::clone(&body));
            thread::spawn(move || {
                let conn = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let Some(_) = read_head(&mut tls) else { return };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                if tls.write_all(head.as_bytes()).is_err() || tls.flush().is_err() {
                    return;
                }
                // One record per piece, all queued before any is sent.
                tls.conn.set_buffer_limit(None);
                let Records::Of(size) = record;
                for piece in body.chunks(size) {
                    if tls.conn.writer().write_all(piece).is_err() {
                        return;
                    }
                }
                let _ = tls.flush();
            });
        }
    });
    port
}

fn tls_client(proxy: Option<&str>) -> Client {
    Client::with_tls_root_for_tests(Box::new(Anonymous), ROOT, proxy)
}

fn blob(port: u16, bytes: &[u8], proxy: Option<&str>) -> Result<Vec<u8>, OciError> {
    let r = Reference::parse(&format!("127.0.0.1:{port}/a/b:1")).unwrap();
    let desc = Descriptor::for_bytes("x", bytes);
    let mut out = Vec::new();
    tls_client(proxy).pull_blob(&r, &desc, &mut out)?;
    Ok(out)
}

/// A `CONNECT` proxy that tunnels every connection to `target`.
fn connect_proxy(target: u16) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let tunnels = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&tunnels);
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let count = Arc::clone(&count);
            thread::spawn(move || {
                if read_head(&mut &client).is_none() {
                    return;
                }
                count.fetch_add(1, Ordering::SeqCst);
                let Ok(upstream) = TcpStream::connect(("127.0.0.1", target)) else {
                    return;
                };
                let _ = (&client).write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
                let (mut up_r, mut up_w) = (upstream.try_clone().unwrap(), upstream);
                let (mut cl_r, mut cl_w) = (client.try_clone().unwrap(), client);
                thread::spawn(move || {
                    let _ = std::io::copy(&mut cl_r, &mut up_w);
                    let _ = up_w.shutdown(std::net::Shutdown::Write);
                });
                let _ = std::io::copy(&mut up_r, &mut cl_w);
            });
        }
    });
    (port, tunnels)
}

#[test]
fn a_blob_over_tls_downloads() {
    let bytes = vec![7u8; 1 << 20];
    let port = tls_registry(bytes.clone(), Records::Of(16 << 10));
    assert_eq!(blob(port, &bytes, None).unwrap(), bytes);
}

#[test]
fn a_blob_in_tiny_tls_records_is_refused_though_its_plaintext_is_small() {
    // 200 000 one-byte records are about 4.4 MB on the wire for 200 000 bytes
    // of body, far past the cap on the socket.
    let bytes = vec![7u8; 200_000];
    let port = tls_registry(bytes.clone(), Records::Of(1));
    assert_refused(&blob(port, &bytes, None));
}

#[test]
fn a_blob_through_a_connect_proxy_downloads() {
    let bytes = vec![7u8; 1 << 20];
    let port = tls_registry(bytes.clone(), Records::Of(16 << 10));
    let (proxy_port, tunnels) = connect_proxy(port);
    let proxy = format!("http://127.0.0.1:{proxy_port}");
    assert_eq!(blob(port, &bytes, Some(&proxy)).unwrap(), bytes);
    assert_eq!(tunnels.load(Ordering::SeqCst), 1, "the proxy was not used");
}

#[test]
fn tiny_tls_records_through_a_connect_proxy_are_refused() {
    let bytes = vec![7u8; 200_000];
    let port = tls_registry(bytes.clone(), Records::Of(1));
    let (proxy_port, tunnels) = connect_proxy(port);
    let proxy = format!("http://127.0.0.1:{proxy_port}");
    assert_refused(&blob(port, &bytes, Some(&proxy)));
    assert_eq!(tunnels.load(Ordering::SeqCst), 1, "the proxy was not used");
}

/// Threat model N5 (TM-37): no option turns off certificate checks. A server
/// whose certificate chains to a root outside the client's trust store is
/// refused, and the same server is served to a client that trusts that root.
#[test]
fn tm37_a_server_with_an_untrusted_certificate_is_refused() {
    let bytes = vec![7u8; 1024];
    let port = tls_registry(bytes.clone(), Records::Of(1024));
    assert_eq!(
        blob(port, &bytes, None).unwrap(),
        bytes,
        "the control client trusts the root"
    );

    let r = Reference::parse(&format!("127.0.0.1:{port}/a/b:1")).unwrap();
    let desc = Descriptor::for_bytes("x", &bytes);
    let mut out = Vec::new();
    let err = Client::with_credentials(Box::new(Anonymous))
        .pull_blob(&r, &desc, &mut out)
        .unwrap_err();
    assert!(matches!(err, OciError::Network { .. }), "{err:?}");
    assert!(out.is_empty(), "no body bytes were accepted");
}
