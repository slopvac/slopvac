//! The plugin host: spawn, handshake, requests with deadlines, shutdown.
//!
//! No async runtime. Each plugin has five threads: a reader that parses
//! frames and routes responses by request id, a writer fed by a bounded
//! channel, a stderr drain, a waiter that reaps the child and a lifetime
//! watchdog. Callers block in `crossbeam_channel::select!` against a
//! wall-clock deadline.
//!
//! Teardown has one owner, the plugin's `process::Group`. The waiter sees the
//! leader exit without reaping it, tears the group down (stop, walk, kill; see
//! `process`), reaps the leader and only then unregisters the group from
//! `kill_all_live`. Every other kill goes through the same group lock and
//! does nothing once the leader is reaped. The
//! waiter drives teardown even if a descendant keeps standard output open and
//! the reader never sees end of stream. Dropping a plugin waits for the reap,
//! up to five seconds after the kill.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, select};
use serde_json::{Value, json};

use crate::budget::RunBudget;
use crate::error::{PluginError, StartError};
use crate::frame::{FrameError, encode_frame, read_frame_with};
use crate::hooks::parse_error_object;
use crate::process::{Group, Leader, TableSource, Teardown, sanitize_display};
use crate::protocol::{
    FEATURE_DOCUMENT_PATH, HookDecl, HookKind, InitializeResult, MIN_PROTOCOL_VERSION, NameVersion,
    PROTOCOL_VERSION, PluginDeclaration,
};
use crate::strict;

/// Body limit that applies until `initialize` completes.
pub const PRE_INIT_MAX_BODY: usize = 1 << 20;
/// Smallest `maxMessageBytes` a plugin may negotiate.
pub const MIN_MESSAGE_BYTES: u64 = 65_536;
/// Largest request id (2^53-1).
pub const MAX_REQUEST_ID: u64 = (1 << 53) - 1;
const STDERR_RING: usize = 64 * 1024;
const STDERR_SHOWN: usize = 4 * 1024;
/// How long a drop waits for a killed leader to be reaped. A leader that is
/// still unreaped after this stays registered for `kill_all_live`.
const REAP_WAIT: Duration = Duration::from_secs(5);
/// How long shutdown waits for the output readers to reach end of stream.
const DRAIN_WAIT: Duration = Duration::from_millis(500);

/// Timers from plugin-protocol.md section 6.3. Zero is invalid.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// `initialize` reply.
    pub initialize: Duration,
    /// Any hook request except a provider.
    pub hook: Duration,
    /// `provider/complete`.
    pub provider: Duration,
    /// Wait for the answer to a cancelled request.
    pub cancel_grace: Duration,
    /// `shutdown` reply.
    pub shutdown: Duration,
    /// Exit after `exit`.
    pub exit: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(10),
            hook: Duration::from_secs(30),
            provider: Duration::from_secs(120),
            cancel_grace: Duration::from_secs(2),
            shutdown: Duration::from_secs(3),
            exit: Duration::from_secs(2),
        }
    }
}

/// Per-run caps from the threat model, section 5.5.
#[derive(Debug, Clone, Copy)]
pub struct RunCaps {
    /// Unsolicited messages and unmatched responses before a kill.
    pub unsolicited_messages: u32,
    /// Bytes read from standard output.
    pub stdout_bytes: u64,
    /// Bytes read from standard error.
    pub stderr_bytes: u64,
    /// Wall-clock time per plugin.
    pub wall_clock: Duration,
}

impl Default for RunCaps {
    fn default() -> Self {
        Self {
            unsolicited_messages: 1000,
            stdout_bytes: 512 << 20,
            stderr_bytes: 128 << 20,
            wall_clock: Duration::from_mins(30),
        }
    }
}

/// What the host offers and proposes.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Host name for `hostInfo`.
    pub host_name: String,
    /// Host version for `hostInfo`.
    pub host_version: String,
    /// Offline mode, from every source (flag, environment, config, sticky state).
    pub offline: bool,
    /// Hook kinds the host can call.
    pub hook_kinds: Vec<HookKind>,
    /// Features the host grants.
    pub features: Vec<String>,
    /// Proposed `maxMessageBytes`.
    pub max_message_bytes: usize,
    /// Proposed `maxBatchDocuments`.
    pub max_batch_documents: usize,
    /// Ceiling for the plugin's `maxInFlight`.
    pub max_in_flight: usize,
    /// Timers.
    pub timeouts: Timeouts,
    /// Run caps.
    pub caps: RunCaps,
    /// Whole-run budget. Clones of one config share it, so every plugin
    /// started from them draws on the same counters.
    pub budget: Arc<RunBudget>,
    /// Test seam: where a teardown reads the process table. `None`, the
    /// default, means `/bin/ps`; production code never sets it.
    #[doc(hidden)]
    pub process_table: Option<TableSource>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            host_name: "slopvac".into(),
            host_version: env!("CARGO_PKG_VERSION").into(),
            offline: false,
            hook_kinds: HookKind::ALL.to_vec(),
            features: vec![FEATURE_DOCUMENT_PATH.into()],
            max_message_bytes: 16 << 20,
            max_batch_documents: 64,
            max_in_flight: 16,
            timeouts: Timeouts::default(),
            caps: RunCaps::default(),
            budget: Arc::default(),
            process_table: None,
        }
    }
}

/// How to start one plugin, from its trusted manifest.
#[derive(Debug, Clone)]
pub struct PluginSpec {
    /// Name used in diagnostics.
    pub name: String,
    /// Absolute path of the program inside the verified tree.
    pub program: PathBuf,
    /// Arguments from the manifest.
    pub args: Vec<String>,
    /// Working directory: the plugin's own installed directory.
    pub cwd: PathBuf,
    /// The hashed declaration.
    pub declaration: PluginDeclaration,
    /// `PATH` for the child. The host's own `PATH` is never inherited.
    pub path_env: String,
    /// Extra variables the manifest names and the user allowed.
    pub env: Vec<(String, String)>,
}

/// The pre-spawn check that backs plugin-protocol.md section 4.3.
///
/// [`crate::trust::TrustGate`] is the production implementation: it recomputes
/// the tree hash and checks the trust hash (`slopvac plugins trust`) and the
/// revocation list before every spawn. Signatures are verified earlier, at sync.
pub trait SpawnGate: Send + Sync {
    /// Refuse the spawn with [`PluginError::Untrusted`] or
    /// [`PluginError::HashMismatch`], or allow it.
    ///
    /// # Errors
    /// Any reason the plugin must not run.
    fn check(&self, spec: &PluginSpec) -> Result<(), PluginError>;
}

/// Allocates request ids: 1 up to a maximum, never reused, never wrapped.
#[derive(Debug)]
pub struct IdAllocator {
    next: AtomicU64,
    max: u64,
}

impl IdAllocator {
    /// Allocator that ends after `max`.
    #[must_use]
    pub fn with_max(max: u64) -> Self {
        Self {
            next: AtomicU64::new(1),
            max,
        }
    }

    /// The next id.
    ///
    /// # Errors
    /// [`PluginError::RequestIdExhausted`] after the maximum.
    pub fn next_id(&self) -> Result<u64, PluginError> {
        self.next
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |c| {
                (c <= self.max).then_some(c + 1)
            })
            .map_err(|_| PluginError::RequestIdExhausted)
    }
}

type Reply = Result<Value, PluginError>;
/// A reply and the moment the reader delivered it.
type Stamped = (Instant, Reply);

struct Pending {
    reply: Sender<Stamped>,
    cancel: Sender<()>,
}

struct Shared {
    name: String,
    group: Arc<Group>,
    pending: Mutex<HashMap<u64, Pending>>,
    dead: Mutex<Option<PluginError>>,
    max_body: AtomicUsize,
    shutting_down: AtomicBool,
    frames: Mutex<Option<Sender<Vec<u8>>>>,
    stderr: Mutex<VecDeque<u8>>,
    stderr_done: Receiver<()>,
    /// Disconnects when the standard output reader has ended.
    reader_done: Receiver<()>,
    caps: RunCaps,
    budget: Arc<RunBudget>,
    exit_timeout: Duration,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// First failure wins. Kills the plugin and fails every pending request.
    fn fail(&self, err: PluginError) -> PluginError {
        let first = {
            let mut dead = lock(&self.dead);
            dead.get_or_insert_with(|| err).clone()
        };
        self.group.kill();
        lock(&self.frames).take();
        let drained: Vec<Pending> = lock(&self.pending).drain().map(|(_, p)| p).collect();
        for p in drained {
            let _ = p.reply.try_send((Instant::now(), Err(first.clone())));
        }
        first
    }

    fn stderr_tail(&self) -> String {
        let ring = lock(&self.stderr);
        let bytes: Vec<u8> = ring.iter().copied().collect();
        let start = bytes.len().saturating_sub(STDERR_SHOWN);
        let text = sanitize_display(&String::from_utf8_lossy(&bytes[start..]));
        let text = text.trim_end();
        if text.is_empty() {
            String::new()
        } else {
            format!("\nstderr:\n{text}")
        }
    }

    /// During shutdown the stream or process ending is expected: fail what is
    /// still waiting, but leave success and the kill to `Plugin::shutdown`.
    fn release_pending(&self) {
        let drained: Vec<Pending> = lock(&self.pending).drain().map(|(_, p)| p).collect();
        for p in drained {
            let _ = p.reply.try_send((
                Instant::now(),
                Err(PluginError::Usage("the plugin has shut down".into())),
            ));
        }
    }

    /// Collect the exit status and stderr tail, fail every pending request.
    fn tear_down_after_exit(&self) {
        let _ = self.stderr_done.recv_timeout(Duration::from_millis(500));
        let status = self
            .group
            .status()
            .unwrap_or_else(|| "status unknown".into());
        self.fail(PluginError::Exited {
            plugin: self.name.clone(),
            status,
            stderr: self.stderr_tail(),
        });
    }

    /// Standard output ended or broke.
    fn on_stream_end(&self) {
        if self.shutting_down.load(Ordering::SeqCst) {
            self.release_pending();
            return;
        }
        // Let the waiter reap the leader first, so the error carries its status.
        self.group.kill();
        let _ = self.group.wait_done(self.exit_timeout);
        self.tear_down_after_exit();
    }

    /// The waiter reaped the child. Independent of pipe end of stream.
    fn on_process_exit(&self) {
        if self.shutting_down.load(Ordering::SeqCst) {
            self.release_pending();
            return;
        }
        self.tear_down_after_exit();
    }

    fn violation(&self, msg: impl Into<String>) {
        self.fail(PluginError::ProtocolViolation(format!(
            "{}: {}",
            self.name,
            msg.into()
        )));
    }

    /// Route one message. `Ok(true)` means a pending request got its reply.
    fn dispatch(&self, msg: &Value, unsolicited: &mut u32) -> Result<bool, String> {
        let obj = msg.as_object().ok_or("message is not a JSON object")?;
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("`jsonrpc` is not \"2.0\"".into());
        }
        let unmatched = |n: &mut u32| {
            *n += 1;
            // Counts even when the message also crosses the plugin's own cap.
            let charged = self.budget.charge_unsolicited();
            if *n > self.caps.unsolicited_messages {
                Err(format!(
                    "more than {} unsolicited messages",
                    self.caps.unsolicited_messages
                ))
            } else {
                charged.map(|()| false)
            }
        };
        if obj.contains_key("method") {
            // v1 has no plugin-to-host requests; notifications are dropped.
            return unmatched(unsolicited);
        }
        let id = match obj.get("id") {
            Some(Value::Null) => return unmatched(unsolicited),
            Some(v) => v
                .as_u64()
                .filter(|&i| (1..=MAX_REQUEST_ID).contains(&i))
                .ok_or("response id is not an integer from 1 to 2^53-1")?,
            None => return Err("message has neither `method` nor `id`".into()),
        };
        let reply = match (obj.get("result"), obj.get("error")) {
            (Some(r), None) => Ok(r.clone()),
            (None, Some(e)) => Err(match parse_error_object(e) {
                Ok(rpc) => PluginError::Rpc(rpc),
                Err(e) => e,
            }),
            _ => return Err("response needs exactly one of `result` and `error`".into()),
        };
        match lock(&self.pending).remove(&id) {
            Some(p) => {
                let _ = p.reply.try_send((Instant::now(), reply));
                Ok(true)
            }
            None => unmatched(unsolicited),
        }
    }
}

/// Counts every byte read from the stream.
struct Counted<R> {
    inner: R,
    read: u64,
    budget: Arc<RunBudget>,
    /// Set when a read broke the whole-run budget.
    breach: Option<String>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read = self.read.saturating_add(n as u64);
        // Charged as bytes leave the pipe: partial frames, read-ahead and
        // failed streams count too.
        if let Err(m) = self.budget.charge_stdout(n as u64) {
            self.breach = Some(m);
            return Err(io::Error::other("whole-run budget exceeded"));
        }
        Ok(n)
    }
}

/// Read and route frames until the stream ends or breaks.
///
/// The first reply is the `initialize` reply. After it the reader stops until
/// `limits_ready` fires, so the next frame meets the negotiated body limit
/// rather than the pre-initialize one (plugin-protocol.md section 2.2). A
/// closed `limits_ready` means the handshake failed and the reader stops.
///
/// The volume cap counts every byte the frames consumed, headers and bodies.
fn reader_loop(shared: &Shared, stdout: impl Read, limits_ready: &Receiver<()>) {
    let mut r = BufReader::new(Counted {
        inner: stdout,
        read: 0,
        budget: Arc::clone(&shared.budget),
        breach: None,
    });
    let mut unsolicited = 0u32;
    let mut negotiated = false;
    loop {
        let frame = read_frame_with(&mut r, &|| shared.max_body.load(Ordering::SeqCst));
        if let Some(m) = r.get_mut().breach.take() {
            return shared.violation(m);
        }
        match frame {
            Ok(None) | Err(FrameError::Io(_)) => return shared.on_stream_end(),
            Err(FrameError::Framing(m)) => return shared.violation(m),
            Ok(Some(body)) => {
                // Bytes buffered for the next frame are not consumed yet.
                let total = r.get_ref().read - r.buffer().len() as u64;
                if total > shared.caps.stdout_bytes {
                    return shared.violation("standard output volume cap reached");
                }
                let msg = match strict::parse(&body) {
                    Ok(v) => v,
                    Err(m) => return shared.violation(m),
                };
                match shared.dispatch(&msg, &mut unsolicited) {
                    Err(m) => return shared.violation(m),
                    Ok(true) if !negotiated => {
                        if limits_ready.recv().is_err() {
                            return;
                        }
                        negotiated = true;
                    }
                    Ok(_) => {}
                }
            }
        }
    }
}

fn stderr_loop(shared: &Shared, mut stderr: impl Read, done: Sender<()>) {
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        match stderr.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total = total.saturating_add(n as u64);
                {
                    let mut ring = lock(&shared.stderr);
                    ring.extend(&buf[..n]);
                    let excess = ring.len().saturating_sub(STDERR_RING);
                    ring.drain(..excess);
                }
                if let Err(m) = shared.budget.charge_stderr(n as u64) {
                    shared.violation(m);
                    break;
                }
                if total > shared.caps.stderr_bytes {
                    shared.fail(PluginError::ProtocolViolation(format!(
                        "{}: standard error volume cap reached",
                        shared.name
                    )));
                    break;
                }
            }
        }
    }
    drop(done);
}

/// A running connection with no hook semantics yet.
struct Conn {
    shared: Arc<Shared>,
    ids: IdAllocator,
    cancel_grace: Duration,
    /// Releases the reader once the negotiated limits are installed.
    limits_ready: Option<Sender<()>>,
    /// Dropping this stops the lifetime watchdog.
    _watchdog_stop: Sender<()>,
}

/// What ended a caller's wait for a reply.
#[derive(Debug)]
enum Wake {
    /// The reply arrived in time, or `None` when the request was dropped without one.
    Reply(Option<Reply>),
    /// The reply arrived, but after the deadline.
    Late(Reply),
    /// `cancel_pending` asked the request to stop.
    Cancel,
    /// The deadline passed with no reply.
    Deadline,
}

/// Wait for a reply, an explicit cancel or the deadline.
///
/// The reader stamps each reply when it delivers it, and that stamp alone
/// decides whether the reply was in time (plugin-protocol.md section 6.3). A
/// caller that wakes late can find the reply, the closed cancel channel and
/// the deadline ready at once, and `select!` picks among them at random, so
/// every branch classifies the reply the same way. Delivering a reply drops
/// the request's cancel sender. A closed cancel channel is therefore no
/// cancel: the reply is already in `reply_rx`.
fn wait_for(reply_rx: &Receiver<Stamped>, cancel_rx: &Receiver<()>, deadline: Instant) -> Wake {
    let classify = |got: Option<Stamped>, timed_out: bool| match got {
        Some((at, r)) if at <= deadline => Wake::Reply(Some(r)),
        Some((_, r)) => Wake::Late(r),
        None if timed_out => Wake::Deadline,
        None => Wake::Reply(None),
    };
    select! {
        recv(reply_rx) -> r => classify(r.ok(), false),
        recv(cancel_rx) -> c => match c {
            Ok(()) => Wake::Cancel,
            Err(_) => classify(reply_rx.try_recv().ok(), false),
        },
        recv(crossbeam_channel::at(deadline)) -> _ => classify(reply_rx.try_recv().ok(), true),
    }
}

impl Conn {
    fn dead(&self) -> Option<PluginError> {
        lock(&self.shared.dead).clone()
    }

    fn send_frame(&self, body: &Value, deadline: Instant) -> Result<(), PluginError> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| PluginError::Usage(format!("cannot encode message: {e}")))?;
        let max = self.shared.max_body.load(Ordering::SeqCst);
        if bytes.len() > max {
            return Err(PluginError::MessageTooLarge(format!(
                "{} bytes exceed the limit of {max}",
                bytes.len()
            )));
        }
        let Some(tx) = lock(&self.shared.frames).clone() else {
            return Err(self
                .dead()
                .unwrap_or_else(|| PluginError::Usage("plugin is closed".into())));
        };
        match tx.send_deadline(encode_frame(&bytes), deadline) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::SendTimeoutError::Timeout(_)) => Err(self
                .shared
                .fail(self.timeout("the write to the plugin blocked"))),
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => Err(self
                .dead()
                .unwrap_or_else(|| PluginError::Usage("plugin is closed".into()))),
        }
    }

    fn timeout(&self, what: &str) -> PluginError {
        PluginError::Timeout {
            plugin: self.shared.name.clone(),
            what: what.to_owned(),
        }
    }

    fn notify(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<(), PluginError> {
        let mut msg = json!({"jsonrpc": "2.0", "method": method});
        if let Some(p) = params {
            msg["params"] = p;
        }
        self.send_frame(&msg, Instant::now() + timeout)
    }

    /// Send a request and wait for its reply.
    ///
    /// With `escalate`, a missed deadline cancels the request and then kills the
    /// plugin when it stays silent. Without it, the caller just gives up.
    fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        escalate: bool,
    ) -> Reply {
        let id = match self.ids.next_id() {
            Ok(id) => id,
            Err(e) => return Err(self.shared.fail(e)),
        };
        let deadline = Instant::now() + timeout;
        let (reply_tx, reply_rx) = bounded::<Stamped>(1);
        let (cancel_tx, cancel_rx) = bounded::<()>(1);
        {
            let mut pending = lock(&self.shared.pending);
            if let Some(e) = self.dead() {
                return Err(e);
            }
            pending.insert(
                id,
                Pending {
                    reply: reply_tx,
                    cancel: cancel_tx,
                },
            );
        }
        let mut msg = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(p) = params {
            msg["params"] = p;
        }
        if let Err(e) = self.send_frame(&msg, deadline) {
            lock(&self.shared.pending).remove(&id);
            return Err(e);
        }
        match wait_for(&reply_rx, &cancel_rx, deadline) {
            Wake::Reply(r) => {
                r.unwrap_or_else(|| Err(self.dead().unwrap_or(PluginError::Cancelled)))
            }
            // The plugin answered, so there is nothing to cancel; the deadline still stands.
            Wake::Late(answer) => Err(self.settle(answer, true, method)),
            Wake::Cancel => self.cancel_and_wait(id, &reply_rx, false, method),
            Wake::Deadline if escalate => self.cancel_and_wait(id, &reply_rx, true, method),
            Wake::Deadline => {
                lock(&self.shared.pending).remove(&id);
                Err(self.timeout(&format!("`{method}` got no reply")))
            }
        }
    }

    /// Send `$/cancelRequest`, then give the plugin the grace period to answer.
    fn cancel_and_wait(
        &self,
        id: u64,
        rx: &Receiver<Stamped>,
        deadline_passed: bool,
        method: &str,
    ) -> Reply {
        let _ = self.notify(
            "$/cancelRequest",
            Some(json!({"id": id})),
            self.cancel_grace,
        );
        let Ok((_, answer)) = rx.recv_timeout(self.cancel_grace) else {
            lock(&self.shared.pending).remove(&id);
            let err = self.timeout(&format!("`{method}` ignored its cancel"));
            return Err(self.shared.fail(err));
        };
        Err(self.settle(answer, deadline_passed, method))
    }

    /// The error for an answer the host discards (plugin-protocol.md section 6.2).
    fn settle(&self, answer: Reply, deadline_passed: bool, method: &str) -> PluginError {
        match answer {
            Err(e) if !matches!(e, PluginError::Rpc(_)) => e,
            _ if deadline_passed => self.timeout(&format!("`{method}` passed its deadline")),
            _ => PluginError::Cancelled,
        }
    }

    fn cancel_all(&self) {
        for p in lock(&self.shared.pending).values() {
            let _ = p.cancel.try_send(());
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // The drop guard of plugin-protocol.md section 3: nothing outlives the host.
        if !self.shared.shutting_down.load(Ordering::SeqCst) {
            self.shared
                .fail(PluginError::Usage("plugin dropped".into()));
        }
        // Blocks while another kill is in flight; does nothing once reaped.
        self.shared.group.kill();
        // The waiter reaps the leader and unregisters the group.
        let _ = self.shared.group.wait_done(REAP_WAIT);
    }
}

/// Limits and capabilities fixed by `initialize`.
#[derive(Debug, Clone)]
pub struct Negotiated {
    /// Body limit, both directions.
    pub max_message_bytes: usize,
    /// Documents per request.
    pub max_batch_documents: usize,
    /// Requests in flight.
    pub max_in_flight: usize,
    /// Hooks the plugin offers, known kinds only.
    pub hooks: Vec<HookDecl>,
    /// Whether `document-path` was granted.
    pub document_path: bool,
    /// Plugin name and version as reported.
    pub info: NameVersion,
}

/// A started, initialised plugin.
pub struct Plugin {
    conn: Conn,
    negotiated: Negotiated,
    inflight_tx: Sender<()>,
    inflight_rx: Receiver<()>,
    timeouts: Timeouts,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("name", &self.conn.shared.name)
            .field("negotiated", &self.negotiated)
            .finish_non_exhaustive()
    }
}

fn prespawn(spec: &PluginSpec, cfg: &HostConfig, gate: &dyn SpawnGate) -> Result<(), PluginError> {
    gate.check(spec)?;
    if spec.declaration.network && cfg.offline {
        return Err(PluginError::Offline(spec.name.clone()));
    }
    for f in &spec.declaration.features.required {
        if !cfg.features.contains(f) {
            return Err(PluginError::RequiredFeatureMissing(f.clone()));
        }
    }
    Ok(())
}

fn check_reply(
    spec: &PluginSpec,
    cfg: &HostConfig,
    reply: &InitializeResult,
) -> Result<Negotiated, PluginError> {
    let mismatch = |m: String| PluginError::DeclarationMismatch(format!("{}: {m}", spec.name));
    let decl = &spec.declaration;
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&reply.protocol_version) {
        return Err(PluginError::ProtocolVersionUnsupported {
            host: format!("{MIN_PROTOCOL_VERSION}..={PROTOCOL_VERSION}"),
            plugin: reply.protocol_version.to_string(),
        });
    }
    let caps = &reply.capabilities;
    if caps.requires_network != decl.network {
        return Err(mismatch(format!(
            "reply says requiresNetwork={}, manifest says network={}",
            caps.requires_network, decl.network
        )));
    }
    for f in &caps.features.required {
        if !cfg.features.contains(f) {
            return Err(PluginError::RequiredFeatureMissing(f.clone()));
        }
        if !decl.features.required.contains(f) {
            return Err(mismatch(format!(
                "feature `{f}` is not required in the manifest"
            )));
        }
    }
    for f in &caps.features.optional {
        if !decl.features.optional.contains(f) {
            return Err(mismatch(format!(
                "feature `{f}` is not optional in the manifest"
            )));
        }
    }
    let mut hooks = Vec::new();
    for h in &caps.hooks {
        let Some(kind) = HookKind::parse(&h.kind) else {
            continue;
        };
        if !cfg.hook_kinds.contains(&kind) {
            continue;
        }
        if !decl.hooks.contains(h) {
            return Err(mismatch(format!(
                "hook {}/{} is not in the manifest",
                h.kind, h.id
            )));
        }
        hooks.push(h.clone());
    }
    let out = |m: String| PluginError::OutputInvalid(format!("{}: {m}", spec.name));
    let l = reply.limits;
    let max_msg = usize::try_from(l.max_message_bytes)
        .ok()
        .filter(|&m| m as u64 >= MIN_MESSAGE_BYTES && m <= cfg.max_message_bytes)
        .ok_or_else(|| {
            out(format!(
                "maxMessageBytes {} is outside the allowed range",
                l.max_message_bytes
            ))
        })?;
    let max_batch = usize::try_from(l.max_batch_documents)
        .ok()
        .filter(|&m| (1..=cfg.max_batch_documents).contains(&m))
        .ok_or_else(|| {
            out(format!(
                "maxBatchDocuments {} is outside the allowed range",
                l.max_batch_documents
            ))
        })?;
    let max_flight = usize::try_from(l.max_in_flight)
        .ok()
        .filter(|&m| (1..=cfg.max_in_flight).contains(&m))
        .ok_or_else(|| {
            out(format!(
                "maxInFlight {} is outside the allowed range",
                l.max_in_flight
            ))
        })?;
    let wants_path = |s: &[String]| s.iter().any(|f| f == FEATURE_DOCUMENT_PATH);
    let document_path = cfg.features.iter().any(|f| f == FEATURE_DOCUMENT_PATH)
        && (wants_path(&caps.features.required) || wants_path(&caps.features.optional));
    Ok(Negotiated {
        max_message_bytes: max_msg,
        max_batch_documents: max_batch,
        max_in_flight: max_flight,
        hooks,
        document_path,
        info: reply.plugin_info.clone(),
    })
}

impl Plugin {
    /// Check the gate, spawn the plugin and run the `initialize` handshake.
    ///
    /// # Errors
    /// Any pre-spawn refusal, spawn failure or handshake failure. On failure
    /// the process group is already dead, and the error carries what its
    /// teardown left undone.
    pub fn start(
        spec: &PluginSpec,
        cfg: &HostConfig,
        gate: &dyn SpawnGate,
    ) -> Result<Self, StartError> {
        if let Some(m) = cfg.budget.breach() {
            return Err(StartError {
                error: PluginError::ProtocolViolation(format!("{}: {m}", spec.name)),
                teardown: Teardown::default(),
            });
        }
        prespawn(spec, cfg, gate).map_err(|error| StartError {
            error,
            teardown: Teardown::default(),
        })?;
        // The breach is published under the lock that guards the spawn, so a
        // start that waited in its gate cannot spawn after a breach.
        let conn = cfg
            .budget
            .admit(|| spawn(spec, cfg))
            .map_err(|m| StartError {
                error: PluginError::ProtocolViolation(format!("{}: {m}", spec.name)),
                teardown: Teardown::default(),
            })??;
        let group = Arc::clone(&conn.shared.group);
        // A failed handshake drops the connection, which kills the group and
        // waits for the reap, so the teardown is final here.
        Self::handshake(conn, spec, cfg).map_err(|error| StartError {
            error,
            teardown: group.teardown(),
        })
    }

    /// Run `initialize` and `initialized` over a fresh connection.
    fn handshake(mut conn: Conn, spec: &PluginSpec, cfg: &HostConfig) -> Result<Self, PluginError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "minProtocolVersion": MIN_PROTOCOL_VERSION,
            "hostInfo": {"name": cfg.host_name, "version": cfg.host_version},
            "capabilities": {
                "hooks": cfg.hook_kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                "features": cfg.features,
            },
            "offline": cfg.offline,
            "limits": {
                "maxMessageBytes": cfg.max_message_bytes,
                "maxBatchDocuments": cfg.max_batch_documents,
            },
        });
        let result = match conn.request("initialize", Some(params), cfg.timeouts.initialize, true) {
            Ok(v) => v,
            Err(PluginError::Rpc(e)) if e.slopvac_code == "protocol-version-unsupported" => {
                return Err(PluginError::ProtocolVersionUnsupported {
                    host: format!("{MIN_PROTOCOL_VERSION}..={PROTOCOL_VERSION}"),
                    plugin: e
                        .data
                        .get("supported")
                        .map_or_else(|| "unknown".into(), Value::to_string),
                });
            }
            Err(e) => return Err(e),
        };
        let reply: InitializeResult = serde_json::from_value(result).map_err(|e| {
            PluginError::OutputInvalid(format!("{}: initialize result: {e}", spec.name))
        })?;
        let negotiated = check_reply(spec, cfg, &reply)?;
        conn.shared
            .max_body
            .store(negotiated.max_message_bytes, Ordering::SeqCst);
        // The reader stopped after the reply; it resumes under the new limit.
        if let Some(ready) = conn.limits_ready.take() {
            let _ = ready.send(());
        }
        conn.notify("initialized", Some(json!({})), cfg.timeouts.initialize)?;
        let (inflight_tx, inflight_rx) = bounded(negotiated.max_in_flight);
        Ok(Self {
            conn,
            negotiated,
            inflight_tx,
            inflight_rx,
            timeouts: cfg.timeouts,
        })
    }

    /// Limits and capabilities fixed by the handshake.
    #[must_use]
    pub fn negotiated(&self) -> &Negotiated {
        &self.negotiated
    }

    /// The plugin name used in diagnostics.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.conn.shared.name
    }

    /// Whether the plugin offers the hook.
    #[must_use]
    pub fn offers(&self, kind: HookKind, id: &str) -> bool {
        self.negotiated
            .hooks
            .iter()
            .any(|h| h.kind == kind.as_str() && h.id == id)
    }

    /// Send one hook request and return its raw `result`.
    ///
    /// Only hooks the plugin offered are called. At most `maxInFlight`
    /// requests run at once; extra callers wait for a slot inside the deadline.
    ///
    /// # Errors
    /// Any [`PluginError`]. A timeout or protocol failure kills the plugin.
    pub fn call(&self, kind: HookKind, id: &str, params: Value) -> Result<Value, PluginError> {
        if !self.offers(kind, id) {
            return Err(PluginError::Usage(format!(
                "{}: hook {}/{id} was not offered",
                self.name(),
                kind.as_str()
            )));
        }
        let timeout = if kind == HookKind::Provider {
            self.timeouts.provider
        } else {
            self.timeouts.hook
        };
        let start = Instant::now();
        match self.inflight_tx.send_timeout((), timeout) {
            Ok(()) => {}
            Err(_) => return Err(self.conn.timeout("no request slot became free")),
        }
        let _slot = Slot(&self.inflight_rx);
        self.conn.request(
            kind.method(),
            Some(params),
            timeout.saturating_sub(start.elapsed()),
            true,
        )
    }

    /// Ask every pending request to stop, for example after a user interrupt.
    ///
    /// Each waiting caller sends `$/cancelRequest`, discards the answer and
    /// returns [`PluginError::Cancelled`]. A plugin that stays silent is killed
    /// after the grace period.
    pub fn cancel_pending(&self) {
        self.conn.cancel_all();
    }

    /// The failure that ended the plugin, if it has ended.
    #[must_use]
    pub fn failure(&self) -> Option<PluginError> {
        self.conn.dead()
    }

    /// Run the shutdown sequence: `shutdown`, `exit`, then a kill after the
    /// exit timer and a wait of up to five seconds for the reap.
    ///
    /// Descendants that left the plugin's process group are killed first,
    /// while the walk can still find them through their parents.
    #[must_use]
    pub fn shutdown(self) -> Shutdown {
        let shared = Arc::clone(&self.conn.shared);
        shared.shutting_down.store(true, Ordering::SeqCst);
        // A descendant that left the group is lost to the final walk once its
        // parent exits, so it goes now. No snapshot is kept across the waits.
        shared.group.kill_strays();
        // A plugin that a failure already ended is not asked.
        let mut acknowledged = false;
        if self.conn.dead().is_none() {
            acknowledged = self
                .conn
                .request("shutdown", None, self.timeouts.shutdown, false)
                .is_ok_and(|v| v.is_null());
            if acknowledged {
                // Before `exit`, so the waiter sees it when the leader leaves.
                shared.group.acknowledge();
            }
            let _ = self.conn.notify("exit", None, self.timeouts.exit);
        }
        // Closing the pipe ends the writer after it flushes `exit`.
        lock(&shared.frames).take();
        // The waiter reaps the leader and kills what is left of its group.
        let exited = shared.group.wait_done(self.timeouts.exit);
        let left = exited && !shared.group.forced();
        shared.group.kill();
        // The waiter reaps a killed leader; its status belongs in the report.
        let _ = shared.group.wait_done(REAP_WAIT);
        // Output that is still in the pipes counts against the caps, so read
        // both streams to the end before the verdict.
        let _ = shared.stderr_done.recv_timeout(DRAIN_WAIT);
        let _ = shared.reader_done.recv_timeout(DRAIN_WAIT);
        let violation = match lock(&shared.dead).as_ref() {
            Some(PluginError::ProtocolViolation(m)) => Some(m.clone()),
            _ => None,
        };
        Shutdown {
            violation,
            acknowledged,
            left,
            status: shared.group.exit_status(),
            teardown: shared.group.teardown(),
        }
    }

    /// What the teardowns of this plugin left undone so far, for example
    /// after a failure killed it.
    #[must_use]
    pub fn teardown(&self) -> Teardown {
        self.conn.shared.group.teardown()
    }
}

/// How [`Plugin::shutdown`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shutdown {
    /// The plugin answered `shutdown` with `null`, and no failure had ended
    /// it before.
    pub acknowledged: bool,
    /// The plugin left on its own, without a kill.
    pub left: bool,
    /// The leader's exit status, or `None` if it was not reaped by the end
    /// of the shutdown or the wait for it failed.
    pub status: Option<ExitStatus>,
    /// What the teardowns left undone.
    pub teardown: Teardown,
    /// The protocol violation that ended the plugin, such as output past a
    /// cap that arrived after the shutdown acknowledgement.
    pub violation: Option<String>,
}

impl Shutdown {
    /// The plugin answered `shutdown`, left on its own with status 0
    /// (plugin-protocol.md section 3), and the teardowns left nothing
    /// running that they found.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.violation.is_none()
            && self.acknowledged
            && self.left
            && self.status.is_some_and(|s| s.success())
            && self.teardown.is_complete()
    }
}

struct Slot<'a>(&'a Receiver<()>);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let _ = self.0.try_recv();
    }
}

/// The lifetime cap (threat model 5.5) holds whatever the plugin does, with or
/// without a request in flight. Dropping the `stop` sender ends the thread.
fn spawn_watchdog(shared: &Arc<Shared>, stop: Receiver<()>, cap: Duration) {
    let shared = Arc::clone(shared);
    thread::spawn(move || {
        if stop.recv_timeout(cap) == Err(crossbeam_channel::RecvTimeoutError::Timeout) {
            shared.fail(PluginError::Timeout {
                plugin: shared.name.clone(),
                what: format!("the wall-clock cap of {cap:?} expired"),
            });
        }
    });
}

fn spawn(spec: &PluginSpec, cfg: &HostConfig) -> Result<Conn, StartError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .env("PATH", &spec.path_env)
        .env("LANG", "C.UTF-8")
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|e| StartError {
        error: PluginError::SpawnFailed(format!("{}: {e}", spec.name)),
        teardown: Teardown::default(),
    })?;
    let group = Group::register(child.id(), cfg.process_table.clone());
    let mut leader = Leader::new(child);
    let (Some(stdin), Some(stdout), Some(stderr)) = leader.take_pipes() else {
        group.finish(leader);
        return Err(StartError {
            error: PluginError::SpawnFailed(format!("{}: missing pipes", spec.name)),
            teardown: group.teardown(),
        });
    };
    let (frames_tx, frames_rx) = bounded::<Vec<u8>>(2 * cfg.max_in_flight + 4);
    let (stderr_done_tx, stderr_done) = bounded::<()>(0);
    let (reader_done_tx, reader_done) = bounded::<()>(0);
    let (limits_ready, limits_rx) = bounded::<()>(1);
    let (watchdog_stop, watchdog_rx) = bounded::<()>(0);
    let shared = Arc::new(Shared {
        name: spec.name.clone(),
        group,
        pending: Mutex::new(HashMap::new()),
        dead: Mutex::new(None),
        max_body: AtomicUsize::new(PRE_INIT_MAX_BODY),
        shutting_down: AtomicBool::new(false),
        frames: Mutex::new(Some(frames_tx)),
        stderr: Mutex::new(VecDeque::new()),
        stderr_done,
        reader_done,
        caps: cfg.caps,
        budget: Arc::clone(&cfg.budget),
        exit_timeout: cfg.timeouts.exit,
    });
    {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            // Kill the group while the exited leader still holds its ids, then reap.
            leader.wait_exit();
            shared.group.finish(leader);
            shared.on_process_exit();
        });
    }
    spawn_watchdog(&shared, watchdog_rx, cfg.caps.wall_clock);
    thread::spawn(move || {
        let mut stdin = stdin;
        for frame in frames_rx {
            if stdin
                .write_all(&frame)
                .and_then(|()| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });
    {
        let shared = Arc::clone(&shared);
        thread::spawn(move || stderr_loop(&shared, stderr, stderr_done_tx));
    }
    {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            let _done = reader_done_tx;
            reader_loop(&shared, stdout, &limits_rx);
        });
    }
    Ok(Conn {
        shared,
        ids: IdAllocator::with_max(MAX_REQUEST_ID),
        cancel_grace: cfg.timeouts.cancel_grace,
        limits_ready: Some(limits_ready),
        _watchdog_stop: watchdog_stop,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_count_up_and_stop_at_the_maximum() {
        let ids = IdAllocator::with_max(3);
        let got: Vec<_> = (0..4).map(|_| ids.next_id()).collect();
        assert_eq!(got[0], Ok(1));
        assert_eq!(got[2], Ok(3));
        assert_eq!(got[3], Err(PluginError::RequestIdExhausted));
        assert_eq!(ids.next_id(), Err(PluginError::RequestIdExhausted));
    }

    #[test]
    fn a_delivered_reply_wins_over_the_closed_cancel_channel() {
        // Delivery sends the reply, then drops the cancel sender, so both
        // channels are ready. `select!` picks at random: 64 rounds would all
        // miss a wrong pick with odds of 2^-64.
        for round in 0..64 {
            let (reply_tx, reply_rx) = bounded::<Stamped>(1);
            let (cancel_tx, cancel_rx) = bounded::<()>(1);
            reply_tx.send((Instant::now(), Ok(json!(round)))).unwrap();
            drop(cancel_tx);
            let far = Instant::now() + Duration::from_secs(60);
            match wait_for(&reply_rx, &cancel_rx, far) {
                Wake::Reply(Some(Ok(v))) => assert_eq!(v, json!(round)),
                other => panic!("round {round}: {other:?}"),
            }
        }
    }

    /// A caller that wakes after its deadline finds the reply, the closed
    /// cancel channel and the deadline all ready. The reply's arrival time
    /// alone decides the outcome, whichever channel `select!` picks.
    fn wake_late_with_reply_at(arrived: Instant, deadline: Instant) -> Vec<Wake> {
        (0..64)
            .map(|round| {
                let (reply_tx, reply_rx) = bounded::<Stamped>(1);
                let (cancel_tx, cancel_rx) = bounded::<()>(1);
                reply_tx.send((arrived, Ok(json!(round)))).unwrap();
                drop(cancel_tx);
                wait_for(&reply_rx, &cancel_rx, deadline)
            })
            .collect()
    }

    #[test]
    fn a_reply_that_arrived_after_the_deadline_is_late_whatever_select_picks() {
        let deadline = Instant::now();
        let arrived = deadline + Duration::from_millis(1);
        for (round, wake) in wake_late_with_reply_at(arrived, deadline)
            .iter()
            .enumerate()
        {
            assert!(matches!(wake, Wake::Late(Ok(_))), "round {round}: {wake:?}");
        }
    }

    #[test]
    fn a_reply_that_arrived_before_the_deadline_counts_when_the_caller_wakes_late() {
        let arrived = Instant::now();
        let deadline = arrived + Duration::from_millis(1);
        std::thread::sleep(Duration::from_millis(5));
        for (round, wake) in wake_late_with_reply_at(arrived, deadline)
            .iter()
            .enumerate()
        {
            assert!(
                matches!(wake, Wake::Reply(Some(Ok(_)))),
                "round {round}: {wake:?}"
            );
        }
    }
}
