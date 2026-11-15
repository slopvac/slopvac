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
use std::sync::{Arc, Mutex, PoisonError};

use crossbeam_channel::{Receiver, Sender};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, MarkupContent,
    MarkupKind, NumberOrString, Position, PositionEncodingKind, PublishDiagnosticsParams, Range,
    SaveOptions, ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri, WorkspaceEdit,
};
use slopvac_model::{PositionEncoding, Severity, SourceIndex};

use crate::analysis::{Analysis, Item, analyze};
use crate::convert::{item_range, span_range, touches};
use crate::linter::{Check, Linter};

/// A failure that ends the server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The connection broke or the client broke the protocol.
    #[error("protocol error: {0}")]
    Protocol(#[from] lsp_server::ProtocolError),
    /// The client sent `initialize` parameters this server cannot read.
    #[error("invalid initialize parameters: {0}")]
    Initialize(#[from] serde_json::Error),
    /// The connection closed while the server was still sending.
    #[error("the client connection closed")]
    Closed,
    /// The lint worker thread panicked.
    #[error("the lint worker panicked")]
    WorkerPanicked,
}

/// The language server.
pub struct Server {
    linter: Arc<dyn Linter>,
    checks: Vec<Arc<dyn Check>>,
}

impl Server {
    /// A server that lints with `linter`.
    #[must_use]
    pub fn new(linter: Arc<dyn Linter>) -> Self {
        Self {
            linter,
            checks: Vec::new(),
        }
    }

    /// Run an extra [`Check`] on every buffer, such as the terms check or front-matter
    /// validation.
    #[must_use]
    pub fn with_check(mut self, check: Arc<dyn Check>) -> Self {
        self.checks.push(check);
        self
    }

    /// Serve one client until it sends `shutdown` and `exit`.
    pub fn serve(self, connection: &Connection) -> Result<(), ServerError> {
        let (id, params) = connection.initialize_start()?;
        let params: InitializeParams = serde_json::from_value(params)?;
        let encoding = negotiate(&params);
        let capabilities = capabilities(encoding);
        let init = serde_json::json!({
            "capabilities": capabilities,
            "serverInfo": ServerInfo { name: "slopvac".to_owned(), version: None },
        });
        connection.initialize_finish(id, init)?;

        let shared = Arc::new(Shared {
            linter: self.linter,
            checks: self.checks,
            encoding,
            docs: Mutex::new(HashMap::new()),
        });
        let (jobs_tx, jobs_rx) = crossbeam_channel::unbounded::<String>();
        let worker = {
            let shared = Arc::clone(&shared);
            let out = connection.sender.clone();
            std::thread::Builder::new()
                .name("slopvac-lsp-lint".to_owned())
                .spawn(move || worker(&shared, &jobs_rx, &out))
                .map_err(|_| ServerError::WorkerPanicked)?
        };
        let result = main_loop(&shared, connection, &jobs_tx);
        drop(jobs_tx);
        worker.join().map_err(|_| ServerError::WorkerPanicked)?;
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
    version: i32,
    text: Arc<str>,
    analysis: Option<Arc<Analysis>>,
}

struct Shared {
    linter: Arc<dyn Linter>,
    checks: Vec<Arc<dyn Check>>,
    encoding: PositionEncoding,
    docs: Mutex<HashMap<String, Doc>>,
}

impl Shared {
    fn docs(&self) -> std::sync::MutexGuard<'_, HashMap<String, Doc>> {
        self.docs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, uri: &str, version: i32, text: String) {
        self.docs().insert(
            uri.to_owned(),
            Doc {
                version,
                text: text.into(),
                analysis: None,
            },
        );
    }

    /// The analysis of the current version of `uri`, linting if none is cached. The lock is
    /// not held while linting.
    fn analysis(&self, uri: &str) -> Option<(Arc<str>, Arc<Analysis>)> {
        let (version, text) = {
            let docs = self.docs();
            let doc = docs.get(uri)?;
            if let Some(done) = &doc.analysis {
                return Some((Arc::clone(&doc.text), Arc::clone(done)));
            }
            (doc.version, Arc::clone(&doc.text))
        };
        let done = Arc::new(analyze(
            self.linter.as_ref(),
            &self.checks,
            &path_of(uri),
            version,
            &text,
        ));
        if let Some(doc) = self.docs().get_mut(uri)
            && doc.version == version
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
            // A newer version arrived while linting; its own job is queued.
            let current = shared.docs().get(&uri).map(|d| d.version);
            if current == Some(analysis.version) {
                publish(
                    out,
                    &uri,
                    Some(analysis.version),
                    shared.diagnostics(&text, &analysis),
                );
            }
        }
    }
}

fn main_loop(
    shared: &Shared,
    connection: &Connection,
    jobs: &Sender<String>,
) -> Result<(), ServerError> {
    for message in &connection.receiver {
        match message {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let response = handle_request(shared, req);
                connection
                    .sender
                    .send(Message::Response(response))
                    .map_err(|_| ServerError::Closed)?;
            }
            Message::Notification(note) => {
                handle_notification(shared, note, jobs, &connection.sender);
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
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
                shared.set(&uri, p.text_document.version, p.text_document.text);
                let _ = jobs.send(uri);
            }
        }
        "textDocument/didChange" => {
            if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(note.params) {
                // Full sync: the last change event holds the whole text.
                if let Some(change) = p.content_changes.into_iter().next_back() {
                    let uri = p.text_document.uri.as_str().to_owned();
                    shared.set(&uri, p.text_document.version, change.text);
                    let _ = jobs.send(uri);
                }
            }
        }
        "textDocument/didSave" => {
            if let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params) {
                let uri = p.text_document.uri.as_str().to_owned();
                // Saving relints even when the text did not change, so a changed config or
                // rule set shows up.
                if let Some(doc) = shared.docs().get_mut(&uri) {
                    doc.analysis = None;
                }
                let _ = jobs.send(uri);
            }
        }
        "textDocument/didClose" => {
            if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) {
                let uri = p.text_document.uri.as_str().to_owned();
                shared.docs().remove(&uri);
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
            Err(e) => invalid(id, &e),
        },
        "textDocument/codeAction" => match serde_json::from_value::<CodeActionParams>(params) {
            Ok(p) => ok(id, &code_actions(shared, &p)),
            Err(e) => invalid(id, &e),
        },
        _ => Response::new_err(
            id,
            ErrorCode::MethodNotFound as i32,
            format!("method not supported: {method}"),
        ),
    }
}

fn ok<T: serde::Serialize>(id: RequestId, result: &T) -> Response {
    Response::new_ok(id, result)
}

fn invalid(id: RequestId, error: &serde_json::Error) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, error.to_string())
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
                format!("Apply slopvac fix: {}", item.code),
                &p.text_document.uri,
                edits,
                &diagnostic,
                true,
            ));
        }
        if let Some(rule) = &item.rule {
            for reason in shared.linter.allow_reasons(rule) {
                let line = range.start.line;
                let at = Position::new(line, 0);
                let directive =
                    format!("<!-- slopvac-allow: rule={rule} reason={reason} -->{newline}");
                actions.push(action(
                    format!("Allow {rule} (reason: {reason})"),
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
