//! A registry that follows a small chunked body with an endless trailer is cut
//! off, whatever the fetch (threat model section 6.4, resource exhaustion).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use slopvac_plugins::oci::{Anonymous, Client, Descriptor, OciError, Reference};

/// Trailer bytes the scripted registry tries to send: far past every cap.
const FLOOD: usize = 96 << 20;

/// A registry that answers every request with `body` as one chunk, then trailer
/// lines until `FLOOD` bytes are out or the client hangs up.
struct Flooder {
    port: u16,
    /// Trailer bytes written, sent once per connection when it ends.
    written: mpsc::Receiver<usize>,
}

fn flooder(body: &'static [u8], content_type: &'static str) -> Flooder {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, written) = mpsc::channel();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let tx = tx.clone();
            thread::spawn(move || {
                let _ = tx.send(serve(conn, body, content_type));
            });
        }
    });
    Flooder { port, written }
}

fn serve(mut conn: TcpStream, body: &[u8], content_type: &str) -> usize {
    // Read the request head.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if conn.read(&mut byte).unwrap_or(0) == 0 {
            return 0;
        }
        head.push(byte[0]);
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        body.len()
    );
    let mut ok = conn.write_all(reply.as_bytes()).is_ok()
        && conn.write_all(body).is_ok()
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

impl Flooder {
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

fn assert_refused(out: &Result<(), OciError>) {
    assert!(
        matches!(out, Err(OciError::TooLarge { .. })),
        "expected TooLarge, got {out:?}"
    );
}

const MANIFEST: &[u8] = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.empty.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":2},"layers":[]}"#;

#[test]
fn a_manifest_followed_by_a_trailer_flood_is_refused() {
    let reg = flooder(MANIFEST, "application/vnd.oci.image.manifest.v1+json");
    assert_refused(&client().fetch_manifest(&reg.reference()).map(|_| ()));
    reg.assert_cut_off();
}

#[test]
fn a_blob_followed_by_a_trailer_flood_is_refused() {
    let reg = flooder(b"hello", "application/octet-stream");
    let desc = Descriptor::for_bytes("x", b"hello");
    let out = client().pull_blob(&reg.reference(), &desc, &mut Vec::new());
    assert_refused(&out);
    reg.assert_cut_off();
}

#[test]
fn a_revocation_sized_blob_followed_by_a_trailer_flood_is_refused() {
    let reg = flooder(b"{}", "application/octet-stream");
    let desc = Descriptor::for_bytes("x", b"{}");
    let out = client()
        .fetch_blob_bytes(&reg.reference(), &desc, 64 << 10)
        .map(|_| ());
    assert_refused(&out);
    reg.assert_cut_off();
}

#[test]
fn a_tag_list_followed_by_a_trailer_flood_is_refused() {
    let reg = flooder(br#"{"name":"a/b","tags":["t1"]}"#, "application/json");
    assert_refused(&client().list_tags(&reg.reference()).map(|_| ()));
    reg.assert_cut_off();
}

#[test]
fn a_chunked_blob_with_a_short_trailer_still_downloads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && conn.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
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
