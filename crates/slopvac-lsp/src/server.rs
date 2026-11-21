//! The server: initialize, the message loop and the handlers.
//!
//! Threads: the caller's thread reads messages and answers requests. One worker thread
//! lints. A change notification stores the new text and queues the document on a
//! `crossbeam-channel`; the worker takes everything queued, lints each document once at its
//! latest version and publishes diagnostics. There is no async runtime.
//!
//! The server asks for full document sync, so a buffer is always the exact text the client
//! holds. Positions are derived from that text with the negotiated encoding every time they
//! are needed and are never stored across versions (`docs/contract/positions.md`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, MarkupContent, MarkupKind, MessageType, NumberOrString, Position,
    PositionEncodingKind, PublishDiagnosticsParams, Range, SaveOptions, ServerCapabilities,
    ServerInfo, ShowMessageParams, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri, WorkspaceEdit,
};
use slopvac_model::{Notice, PositionEncoding, Severity, SourceIndex};

use slopvac_messages::render;

use crate::analysis::{Analysis, Item, analyze};
use crate::convert::{item_range, span_range, touches};
use crate::linter::{Check, ConfigFailure, Linter};
use crate::messages::{LspMessage, ServerError};

/// How long a clean exit waits for queued diagnostics to reach the client.
const FLUSH_WAIT: Duration = Duration::from_secs(5);

/// How a served session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The client sent `shutdown`, then `exit`. The process exits with status 0.
    AfterShutdown,
    /// The client sent `exit` with no `shutdown`, or went away. The process exits with
    /// status 1, as the protocol says.
    WithoutShutdown,
}

/// The language server.
pub struct Server {
    linter: Arc<dyn Linter>,
    checks: Vec<Arc<dyn Check>>,
    hook: Option<PublishHook>,
    startup: Vec<Notice>,
}

/// Called with the document uri while the document lock is held, after a diagnostics
/// message was found fresh and before it is queued.
type PublishHook = Arc<dyn Fn(&str) + Send + Sync>;

impl Server {
    /// A server that lints with `linter`.
    #[must_use]
    pub fn new(linter: Arc<dyn Linter>) -> Self {
        Self {
            linter,
            checks: Vec::new(),
            hook: None,
            startup: Vec::new(),
        }
    }

    /// Show `notices` to the client once, as `window/showMessage` warnings, right after the
    /// handshake. They are the facts about the config the linter was built from, such as
    /// a rule id that no rule has. Linting goes on.
    #[must_use]
    pub fn with_startup_notices(mut self, notices: Vec<Notice>) -> Self {
        self.startup = notices;
        self
    }

    /// Run `hook` with the document uri between the freshness check and queueing a
    /// diagnostics message, with the document lock held. Tests use it to interleave a close
    /// at the worst moment.
    #[doc(hidden)]
    #[must_use]
    pub fn with_pre_publish_hook(mut self, hook: PublishHook) -> Self {
        self.hook = Some(hook);
        self
    }

    /// Run an extra [`Check`] on every buffer, such as the terms check or front-matter
    /// validation.
    #[must_use]
    pub fn with_check(mut self, check: Arc<dyn Check>) -> Self {
        self.checks.push(check);
        self
    }

    /// Serve one client until it sends `shutdown` and `exit`.
    pub fn serve(self, connection: &Connection) -> Result<Exit, ServerError> {
        let (id, params) = connection
            .initialize_start()
            .map_err(ServerError::protocol)?;
        let params: InitializeParams =
            serde_json::from_value(params).map_err(|e| ServerError::initialize(&e))?;
        let encoding = negotiate(&params);
        let can_watch = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|w| w.dynamic_registration)
            .unwrap_or(false);
        let capabilities = capabilities(encoding);
        let init = serde_json::json!({
            "capabilities": capabilities,
            "serverInfo": ServerInfo { name: "slopvac".to_owned(), version: None },
        });
        connection
            .initialize_finish(id, init)
            .map_err(ServerError::protocol)?;

        let shared = Arc::new(Shared {
            linter: self.linter,
            checks: self.checks,
            encoding,
            docs: Mutex::new(HashMap::new()),
            next_generation: AtomicU64::new(1),
            hook: self.hook,
            config_shown: Mutex::new(None),
        });
        // Diagnostics are queued here under the document lock, in the order the lock
        // decided, and a writer thread sends them. The transport may block when the client
        // reads slowly, and that must never hold up the document lock.
        let (queue_tx, queue_rx) = crossbeam_channel::unbounded::<Message>();
        let transport = connection.sender.clone();
        let (flushed_tx, flushed) = crossbeam_channel::bounded::<()>(1);
        // Never joined: a client that stopped reading blocks it for good.
        std::thread::Builder::new()
            .name("slopvac-lsp-write".to_owned())
            .spawn(move || {
                for message in queue_rx {
                    if transport.send(message).is_err() {
                        break;
                    }
                }
                let _ = flushed_tx.send(());
            })
            .map_err(|_| ServerError::WorkerPanicked)?;
        let (jobs_tx, jobs_rx) = crossbeam_channel::unbounded::<String>();
        let worker = {
            let shared = Arc::clone(&shared);
            let out = queue_tx.clone();
            std::thread::Builder::new()
                .name("slopvac-lsp-lint".to_owned())
                .spawn(move || worker(&shared, &jobs_rx, &out))
                .map_err(|_| ServerError::WorkerPanicked)?
        };
        // `initialize_finish` has taken the client's `initialized`, so the client is ready.
        for notice in &self.startup {
            notify(
                &queue_tx,
                "window/showMessage",
                &ShowMessageParams {
                    typ: MessageType::WARNING,
                    message: notice.message.as_str().to_owned(),
                },
            );
        }
        if can_watch {
            watch_config(&shared, &queue_tx);
        }
        let result = main_loop(&shared, connection, &jobs_tx, &queue_tx);
        drop(jobs_tx);
        worker.join().map_err(|_| ServerError::WorkerPanicked)?;
        drop(queue_tx);
        if result.is_ok() {
            // Give the writer time to flush what is queued. A client that stopped reading
            // after `shutdown` must not keep the process alive, so the wait is bounded and
            // the writer is left to end with the process. After an error the transport may
            // be stuck, so nothing waits for it.
            let _ = flushed.recv_timeout(FLUSH_WAIT);
        }
        result
    }
}

fn negotiate(params: &InitializeParams) -> PositionEncoding {
    let offered: Option<Vec<&str>> = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref())
        .map(|kinds| kinds.iter().map(PositionEncodingKind::as_str).collect());
    PositionEncoding::negotiate(offered.as_deref())
}

fn capabilities(encoding: PositionEncoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(PositionEncodingKind::new(encoding.name())),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
                ..TextDocumentSyncOptions::default()
            },
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    }
}

struct Doc {
    /// Identifies this text of this open document. Every open, change and save takes a new
    /// one, so a close and reopen at the same client version is told apart from the old
    /// document. Results are cached and published only for the generation they linted.
    generation: u64,
    version: i32,
    text: Arc<str>,
    analysis: Option<Arc<Analysis>>,
}

struct Shared {
    linter: Arc<dyn Linter>,
    checks: Vec<Arc<dyn Check>>,
    encoding: PositionEncoding,
    /// Held while a diagnostics message is sent, so a document cannot close between the
    /// freshness check and the publish.
    docs: Mutex<HashMap<String, Doc>>,
    next_generation: AtomicU64,
    hook: Option<PublishHook>,
    /// The uri that carries the config failure diagnostic, while one is shown.
    config_shown: Mutex<Option<String>>,
}

impl Shared {
    fn docs(&self) -> std::sync::MutexGuard<'_, HashMap<String, Doc>> {
        self.docs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, uri: &str, version: i32, text: String) {
        self.docs().insert(
            uri.to_owned(),
            Doc {
                generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
                version,
                text: text.into(),
                analysis: None,
            },
        );
    }

    /// Drop the cached result of `uri` and start a new generation, so a lint that is still
    /// running for the old one is discarded.
    fn invalidate(&self, uri: &str) {
        if let Some(doc) = self.docs().get_mut(uri) {
            doc.generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
            doc.analysis = None;
        }
    }

    /// Drop every cached result and start a new generation for each open document. Returns
    /// their uris, to be queued for linting.
    fn invalidate_all(&self) -> Vec<String> {
        let mut docs = self.docs();
        docs.iter_mut()
            .map(|(uri, doc)| {
                doc.generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
                doc.analysis = None;
                uri.clone()
            })
            .collect()
    }

    /// Whether `uri` is a config file: a `slopvac.toml`, or a file the linter read.
    fn is_config(&self, uri: &str) -> bool {
        let path = path_of(uri);
        path.rsplit('/').next() == Some(CONFIG_NAME) || self.linter.watched().contains(&path)
    }

    /// The analysis of the current version of `uri`, linting if none is cached. The lock is
    /// not held while linting.
    fn analysis(&self, uri: &str) -> Option<(Arc<str>, Arc<Analysis>)> {
        let (generation, version, text) = {
            let docs = self.docs();
            let doc = docs.get(uri)?;
            if let Some(done) = &doc.analysis {
                return Some((Arc::clone(&doc.text), Arc::clone(done)));
            }
            (doc.generation, doc.version, Arc::clone(&doc.text))
        };
        let done = Arc::new(analyze(
            self.linter.as_ref(),
            &self.checks,
            &path_of(uri),
            generation,
            version,
            &text,
        ));
        if let Some(doc) = self.docs().get_mut(uri)
            && doc.generation == generation
        {
            doc.analysis = Some(Arc::clone(&done));
        }
        Some((text, done))
    }

    fn diagnostics(&self, text: &str, analysis: &Analysis) -> Vec<Diagnostic> {
        let index = SourceIndex::buffer(text);
        analysis
            .items
            .iter()
            .filter_map(|item| Some(diagnostic(item, item_range(&index, item, self.encoding)?)))
            .collect()
    }
}

/// The file name of the project config.
const CONFIG_NAME: &str = "slopvac.toml";

/// Ask the client to tell us when a config file changes on disk: any `slopvac.toml`, and
/// each file the linter read, which can sit outside the project through `extends`.
fn watch_config(shared: &Shared, out: &Sender<Message>) {
    let globs = std::iter::once(format!("**/{CONFIG_NAME}")).chain(shared.linter.watched());
    let watchers: Vec<_> = globs
        .map(|glob| serde_json::json!({"globPattern": glob}))
        .collect();
    let request = Request::new(
        RequestId::from("slopvac-watch-config".to_owned()),
        "client/registerCapability".to_owned(),
        serde_json::json!({"registrations": [{
            "id": "slopvac-watch-config",
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": {"watchers": watchers},
        }]}),
    );
    let _ = out.send(Message::Request(request));
}

/// Read the config again, show what that found, and lint every open document again.
fn reload(shared: &Shared, jobs: &Sender<String>, out: &Sender<Message>) {
    let reloaded = shared.linter.reload();
    for notice in &reloaded.notices {
        notify(
            out,
            "window/showMessage",
            &ShowMessageParams {
                typ: MessageType::WARNING,
                message: notice.message.as_str().to_owned(),
            },
        );
    }
    show_config_failure(shared, reloaded.failure.as_ref(), out);
    for uri in shared.invalidate_all() {
        let _ = jobs.send(uri);
    }
}

/// Publish the config failure on the file that failed, and clear the one shown before.
fn show_config_failure(shared: &Shared, failure: Option<&ConfigFailure>, out: &Sender<Message>) {
    let mut shown = shared
        .config_shown
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let uri = failure.map(|f| format!("file://{}", f.path));
    if let Some(old) = shown.take()
        && uri.as_ref() != Some(&old)
    {
        publish(out, &old, None, Vec::new());
    }
    let (Some(failure), Some(uri)) = (failure, uri) else {
        return;
    };
    let (line, column) = failure.position.unwrap_or((0, 0));
    let at = Position::new(line, column);
    publish(
        out,
        &uri,
        None,
        vec![Diagnostic {
            range: Range::new(at, at),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String(failure.code.clone())),
            source: Some("slopvac".to_owned()),
            message: failure.message.clone(),
            ..Diagnostic::default()
        }],
    );
    *shown = Some(uri);
}

fn path_of(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_owned()
}

fn notify<P: serde::Serialize>(out: &Sender<Message>, method: &str, params: &P) -> bool {
    let note = Notification::new(method.to_owned(), params);
    out.send(Message::Notification(note)).is_ok()
}

fn publish(out: &Sender<Message>, uri: &str, version: Option<i32>, diagnostics: Vec<Diagnostic>) {
    let Ok(uri) = uri.parse::<Uri>() else { return };
    notify(
        out,
        "textDocument/publishDiagnostics",
        &PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        },
    );
}

/// Lints queued documents. It exits when every sender is gone.
fn worker(shared: &Shared, jobs: &Receiver<String>, out: &Sender<Message>) {
    while let Ok(first) = jobs.recv() {
        let mut batch: Vec<String> = vec![first];
        let mut seen: HashSet<String> = batch.iter().cloned().collect();
        for uri in jobs.try_iter() {
            if seen.insert(uri.clone()) {
                batch.push(uri);
            }
        }
        for uri in batch {
            let Some((text, analysis)) = shared.analysis(&uri) else {
                continue;
            };
            let diagnostics = shared.diagnostics(&text, &analysis);
            // Check and queue under one lock: a close that follows queues its clear after
            // this message, and a close that came first has removed the document. Only the
            // queue is touched here, never the transport.
            let docs = shared.docs();
            if docs
                .get(&uri)
                .is_some_and(|d| d.generation == analysis.generation)
            {
                if let Some(hook) = &shared.hook {
                    hook(&uri);
                }
                publish(out, &uri, Some(analysis.version), diagnostics);
            }
        }
    }
}

fn main_loop(
    shared: &Shared,
    connection: &Connection,
    jobs: &Sender<String>,
    queue: &Sender<Message>,
) -> Result<Exit, ServerError> {
    for message in &connection.receiver {
        match message {
            Message::Request(req) => {
                if connection
                    .handle_shutdown(&req)
                    .map_err(ServerError::protocol)?
                {
                    return Ok(Exit::AfterShutdown);
                }
                let response = handle_request(shared, req);
                connection
                    .sender
                    .send(Message::Response(response))
                    .map_err(|_| ServerError::Closed)?;
            }
            Message::Notification(note) if note.method == "exit" => {
                return Ok(Exit::WithoutShutdown);
            }
            Message::Notification(note) => {
                handle_notification(shared, note, jobs, queue);
            }
            Message::Response(_) => {}
        }
    }
    Ok(Exit::WithoutShutdown)
}

fn handle_notification(
    shared: &Shared,
    note: Notification,
    jobs: &Sender<String>,
    out: &Sender<Message>,
) {
    match note.method.as_str() {
        "textDocument/didOpen" => {
            if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) {
                let uri = p.text_document.uri.as_str().to_owned();
                // A config file is not prose. Its diagnostics come from the reload.
                if shared.is_config(&uri) {
                    return;
                }
                shared.set(&uri, p.text_document.version, p.text_document.text);
                let _ = jobs.send(uri);
            }
        }
        "textDocument/didChange" => {
            if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(note.params) {
                // Full sync: the last change event holds the whole text.
                if let Some(change) = p.content_changes.into_iter().next_back() {
                    let uri = p.text_document.uri.as_str().to_owned();
                    if shared.is_config(&uri) {
                        return;
                    }
                    shared.set(&uri, p.text_document.version, change.text);
                    let _ = jobs.send(uri);
                }
            }
        }
        "textDocument/didSave" => {
            if let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params) {
                let uri = p.text_document.uri.as_str().to_owned();
                if shared.is_config(&uri) {
                    reload(shared, jobs, out);
                    return;
                }
                // Saving relints even when the text did not change, so a changed config or
                // rule set shows up.
                shared.invalidate(&uri);
                let _ = jobs.send(uri);
            }
        }
        "workspace/didChangeWatchedFiles" => {
            if let Ok(p) = serde_json::from_value::<DidChangeWatchedFilesParams>(note.params)
                && p.changes.iter().any(|c| shared.is_config(c.uri.as_str()))
            {
                reload(shared, jobs, out);
            }
        }
        "textDocument/didClose" => {
            if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) {
                let uri = p.text_document.uri.as_str().to_owned();
                let mut docs = shared.docs();
                docs.remove(&uri);
                publish(out, &uri, None, Vec::new());
            }
        }
        _ => {}
    }
}

fn handle_request(shared: &Shared, req: Request) -> Response {
    let Request { id, method, params } = req;
    match method.as_str() {
        "textDocument/hover" => match serde_json::from_value::<HoverParams>(params) {
            Ok(p) => ok(id, &hover(shared, &p)),
            Err(e) => invalid(id, "textDocument/hover", &e),
        },
        "textDocument/codeAction" => match serde_json::from_value::<CodeActionParams>(params) {
            Ok(p) => ok(id, &code_actions(shared, &p)),
            Err(e) => invalid(id, "textDocument/codeAction", &e),
        },
        _ => Response::new_err(
            id,
            ErrorCode::MethodNotFound as i32,
            render(&LspMessage::MethodUnsupported { method }),
        ),
    }
}

fn ok<T: serde::Serialize>(id: RequestId, result: &T) -> Response {
    Response::new_ok(id, result)
}

/// An invalid-params response. The parser's text quotes the rejected value, so the message
/// carries only its category and position.
fn invalid(id: RequestId, method: &str, error: &serde_json::Error) -> Response {
    Response::new_err(
        id,
        ErrorCode::InvalidParams as i32,
        render(&LspMessage::params_invalid(method, error)),
    )
}

fn hover(shared: &Shared, p: &HoverParams) -> Option<Hover> {
    let at = p.text_document_position_params.position;
    let uri = p.text_document_position_params.text_document.uri.as_str();
    let (text, analysis) = shared.analysis(uri)?;
    let index = SourceIndex::buffer(&text);
    let probe = Range::new(at, at);
    let mut sections = Vec::new();
    let mut range = None;
    for item in &analysis.items {
        let Some(r) = item_range(&index, item, shared.encoding) else {
            continue;
        };
        // A whole-document item has no extent to hover over.
        if item.span.is_some() && touches(r, probe) {
            sections.push(item.hover.clone());
            range.get_or_insert(r);
        }
    }
    if sections.is_empty() {
        return None;
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: sections.join("\n\n---\n\n"),
        }),
        range,
    })
}

fn code_actions(shared: &Shared, p: &CodeActionParams) -> Vec<CodeActionOrCommand> {
    if let Some(only) = &p.context.only
        && !only.iter().any(|k| {
            k.as_str().is_empty() || CodeActionKind::QUICKFIX.as_str().starts_with(k.as_str())
        })
    {
        return Vec::new();
    }
    let Some((text, analysis)) = shared.analysis(p.text_document.uri.as_str()) else {
        return Vec::new();
    };
    let index = SourceIndex::buffer(&text);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut actions = Vec::new();
    for item in &analysis.items {
        let Some(range) = item_range(&index, item, shared.encoding) else {
            continue;
        };
        if !touches(range, p.range) {
            continue;
        }
        let diagnostic = diagnostic(item, range);
        if let Some(edits) = fix_edits(&index, item, shared.encoding) {
            actions.push(action(
                render(&LspMessage::ActionFix {
                    code: item.code.clone(),
                }),
                &p.text_document.uri,
                edits,
                &diagnostic,
                true,
            ));
        }
        if let Some(rule) = &item.rule {
            let line = range.start.line;
            let site = line_start_offset(&text, line);
            for reason in shared.linter.allow_reasons(
                &path_of(p.text_document.uri.as_str()),
                &text,
                site,
                rule,
            ) {
                let at = Position::new(line, 0);
                let directive =
                    // ast-grep-ignore: user-text-literal
                    format!("<!-- slopvac-allow: rule={rule} reason={reason} -->{newline}");
                actions.push(action(
                    render(&LspMessage::ActionAllow {
                        rule: rule.to_string(),
                        reason,
                    }),
                    &p.text_document.uri,
                    vec![TextEdit::new(Range::new(at, at), directive)],
                    &diagnostic,
                    false,
                ));
            }
        }
    }
    actions
}

/// Byte offset of the start of zero-based `line`. Lines end at LF, CRLF or a lone CR.
fn line_start_offset(text: &str, line: u32) -> usize {
    let bytes = text.as_bytes();
    let (mut start, mut i, mut seen) = (0, 0, 0);
    while seen < line && i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                seen += 1;
                start = i + 1;
            }
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                seen += 1;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    start
}

fn fix_edits(
    index: &SourceIndex<'_>,
    item: &Item,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    item.fix
        .as_ref()?
        .iter()
        .map(|e| {
            Some(TextEdit::new(
                span_range(index, e.span, encoding)?,
                e.text.clone(),
            ))
        })
        .collect()
}

fn action(
    title: String,
    uri: &Uri,
    edits: Vec<TextEdit>,
    diagnostic: &Diagnostic,
    preferred: bool,
) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(uri.clone(), edits)])),
            ..WorkspaceEdit::default()
        }),
        is_preferred: Some(preferred),
        ..CodeAction::default()
    })
}

fn diagnostic(item: &Item, range: Range) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(match item.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Suggestion => DiagnosticSeverity::INFORMATION,
        }),
        code: Some(NumberOrString::String(item.code.clone())),
        source: Some("slopvac".to_owned()),
        message: item.message.clone(),
        ..Diagnostic::default()
    }
}
