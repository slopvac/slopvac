//! A scripted HTTP/1.1 server for the OCI client tests.
#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "test support: each test binary uses a subset of the helpers; test support: items are shared across test binaries; test crate: nothing public to document"
)]

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: vec![],
            body: vec![],
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn body(mut self, b: impl Into<Vec<u8>>) -> Self {
        self.body = b.into();
        self
    }
    pub fn json(self, v: &serde_json::Value) -> Self {
        self.header("Content-Type", "application/json")
            .body(v.to_string())
    }
}

pub struct Server {
    pub addr: SocketAddr,
    pub log: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub fn start(handler: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log2 = Arc::clone(&log);
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log2);
                thread::spawn(move || serve(conn, &*handler, &log));
            }
        });
        Self { addr, log }
    }

    pub fn registry(&self) -> String {
        format!("127.0.0.1:{}", self.addr.port())
    }

    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }

    pub fn count(&self, path_prefix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.path.starts_with(path_prefix))
            .count()
    }
}

/// Wrap `inner` in a registry that enforces authentication like GHCR does.
///
/// `/token` accepts a request when it carries `credentials` as Basic auth (or,
/// with `None`, any request) and answers with the token `<name>|<scope>`.
/// Every other path needs a Bearer token whose scope names the repository and
/// the action the method needs (`pull`, plus `push` for writes); otherwise the
/// wrapper answers 401 with a challenge that points at its own `/token`.
pub fn with_auth(
    name: &'static str,
    credentials: Option<(&'static str, &'static str)>,
    inner: impl Fn(&Request) -> Response + Send + Sync + 'static,
) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    move |req| {
        let host = req.header("host").unwrap_or_default().to_owned();
        if let Some(query) = req.path.strip_prefix("/token?") {
            let scope = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("scope="))
                .unwrap_or("")
                .replace("%3A", ":")
                .replace("%2F", "/")
                .replace("%2C", ",");
            let ok = credentials.is_none_or(|(u, p)| {
                req.header("authorization")
                    == Some(&format!(
                        "Basic {}",
                        base64_std(format!("{u}:{p}").as_bytes())
                    ))
            });
            if !ok {
                return Response::new(401);
            }
            return Response::new(200)
                .json(&serde_json::json!({"token": format!("{name}|{scope}"), "expires_in": 300}));
        }
        let repo = req
            .path
            .strip_prefix("/v2/")
            .and_then(|p| {
                ["/blobs/", "/manifests/", "/tags/", "/referrers/"]
                    .iter()
                    .find_map(|m| p.split_once(m).map(|(r, _)| r))
            })
            .unwrap_or("");
        let write = matches!(req.method.as_str(), "PUT" | "POST" | "PATCH" | "DELETE");
        let actions = if write { "pull,push" } else { "pull" };
        let prefix = format!("Bearer {name}|repository:{repo}:");
        let granted = req
            .header("authorization")
            .and_then(|h| h.strip_prefix(prefix.as_str()))
            .is_some_and(|granted| {
                granted
                    .split(',')
                    .any(|g| g == if write { "push" } else { "pull" })
            });
        if !granted {
            let challenge = format!(
                "Bearer realm=\"http://{host}/token\",service=\"reg\",scope=\"repository:{repo}:{actions}\""
            );
            return Response::new(401).header("WWW-Authenticate", &challenge);
        }
        inner(req)
    }
}

pub fn base64_std(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn serve(
    conn: std::net::TcpStream,
    handler: &dyn Fn(&Request) -> Response,
    log: &Mutex<Vec<Request>>,
) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut out = conn;
    {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
            return;
        };
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 {
                return;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.push((k.trim().to_owned(), v.trim().to_owned()));
            }
        }
        let len = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let req = Request {
            method: method.to_owned(),
            path: path.to_owned(),
            headers,
            body,
        };
        log.lock().unwrap().push(req.clone());
        let resp = handler(&req);
        let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", resp.status);
        for (k, v) in &resp.headers {
            let _ = write!(head, "{k}: {v}\r\n");
        }
        let _ = write!(head, "Content-Length: {}\r\n\r\n", resp.body.len());
        let _ = out.write_all(head.as_bytes());
        if req.method != "HEAD" {
            let _ = out.write_all(&resp.body);
        }
        let _ = out.flush();
    }
}
