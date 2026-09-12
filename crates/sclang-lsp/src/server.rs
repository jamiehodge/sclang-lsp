//! The server: state, capabilities, and the event loop.
//!
//! One thread owns all mutable state and answers every request, so there are
//! no locks on the request path. The only other thread scans the class library
//! at startup and hands over a finished index, because that scan takes long
//! enough to be felt if it happened before `initialize` returned.

use crate::documents::{uri_to_path, DocumentStore};
use crate::features;
use crate::line_index::PositionEncoding;
use crate::locations::Resolver;
use crate::workspace::{build_index, default_roots, IndexStats};

use crossbeam_channel::{Receiver, Sender};
use lsp_server::{Connection, ExtractError, Message, Notification, Request, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::*;
use sclang_index::SymbolIndex;
use std::path::PathBuf;

/// Everything the request path reads.
pub struct Server {
    docs: DocumentStore,
    index: SymbolIndex,
    enc: PositionEncoding,
    sender: Sender<Message>,
}

/// What the background scan sends back when it finishes.
struct Scan {
    index: SymbolIndex,
    stats: IndexStats,
}

impl Server {
    /// Run the server to completion over an already-connected transport.
    pub fn run(connection: Connection) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        let (id, params) = connection.initialize_start()?;
        let params: InitializeParams = serde_json::from_value(params)?;

        let enc = negotiate_encoding(&params);
        let roots = roots_from(&params);

        connection.initialize_finish(id, serde_json::to_value(initialize_result(enc))?)?;

        // Only now, with the editor unblocked, go and read the class library.
        let (scan_tx, scan_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let (index, stats) = build_index(&roots);
            let _ = scan_tx.send(Scan { index, stats });
        });

        let mut server = Server {
            docs: DocumentStore::default(),
            index: SymbolIndex::default(),
            enc,
            sender: connection.sender.clone(),
        };
        server.event_loop(&connection, scan_rx)
    }

    fn event_loop(
        &mut self,
        connection: &Connection,
        scan_rx: Receiver<Scan>,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        loop {
            crossbeam_channel::select! {
                recv(connection.receiver) -> msg => {
                    let Ok(msg) = msg else { return Ok(()) };
                    match msg {
                        Message::Request(req) => {
                            if connection.handle_shutdown(&req)? {
                                return Ok(());
                            }
                            self.request(req);
                        }
                        Message::Notification(note) => self.notification(note),
                        // Responses to requests we made. The server asks the
                        // client nothing yet, so there is nothing to match up.
                        Message::Response(_) => {}
                    }
                }
                recv(scan_rx) -> scan => {
                    if let Ok(scan) = scan {
                        self.install(scan);
                    }
                }
            }
        }
    }

    /// Adopt the scanned index, then re-apply every open buffer on top.
    ///
    /// An open file may have unsaved edits, and the scan read it from disk, so
    /// the buffer has to win or symbols would silently go stale on startup.
    fn install(&mut self, scan: Scan) {
        self.index = scan.index;
        for (uri, doc) in self.docs.iter() {
            self.index.index_file(&uri_to_path(uri), &doc.text);
        }
        let s = scan.stats;
        let message = format!(
            "indexed {} files: {} classes, {} methods{}",
            s.files,
            s.classes,
            s.methods,
            if s.unreadable > 0 {
                format!(" ({} not valid UTF-8, skipped)", s.unreadable)
            } else {
                String::new()
            }
        );
        log(&message);
        // Also tell the editor. It shows up in the client's LSP log, and it
        // is the one observable moment when answers stop being partial.
        self.send(Message::Notification(Notification::new(
            notification::LogMessage::METHOD.to_string(),
            LogMessageParams {
                typ: MessageType::INFO,
                message,
            },
        )));
    }

    // ---- requests ----------------------------------------------------

    fn request(&mut self, req: Request) {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            request::Completion::METHOD => self.handle::<request::Completion>(req, |s, p| {
                let (doc, offset) = s.locate(&p.text_document_position)?;
                Some(features::completion::completion(doc, &s.index, offset))
            }),
            request::SignatureHelpRequest::METHOD => {
                self.handle::<request::SignatureHelpRequest>(req, |s, p| {
                    let (doc, offset) = s.locate(&p.text_document_position_params)?;
                    features::signature_help::signature_help(doc, &s.index, offset)
                })
            }
            request::HoverRequest::METHOD => self.handle::<request::HoverRequest>(req, |s, p| {
                let (doc, offset) = s.locate(&p.text_document_position_params)?;
                features::hover::hover(doc, &s.index, offset, s.enc)
            }),
            request::GotoDefinition::METHOD => {
                self.handle::<request::GotoDefinition>(req, |s, p| {
                    let (doc, offset) = s.locate(&p.text_document_position_params)?;
                    let resolver = Resolver::new(&s.docs, s.enc);
                    features::goto::goto_definition(doc, &s.index, offset, &resolver)
                })
            }
            request::DocumentSymbolRequest::METHOD => self
                .handle::<request::DocumentSymbolRequest>(req, |s, p| {
                    let uri = p.text_document.uri;
                    let doc = s.docs.get(&uri)?;
                    Some(DocumentSymbolResponse::Nested(
                        features::symbols::document_symbols(&uri, doc, s.enc),
                    ))
                }),
            request::WorkspaceSymbolRequest::METHOD => self
                .handle::<request::WorkspaceSymbolRequest>(req, |s, p| {
                    let resolver = Resolver::new(&s.docs, s.enc);
                    Some(WorkspaceSymbolResponse::Flat(
                        features::symbols::workspace_symbols(&s.index, &p.query, &resolver),
                    ))
                }),
            _ => {
                self.send(Response::new_err(
                    id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("unhandled method: {}", req.method),
                ));
                return;
            }
        };
        if let Some(response) = result {
            self.send(response);
        }
    }

    /// Deserialize, dispatch, and serialize one request.
    ///
    /// Every request this server answers has an optional result, so a handler
    /// can bail with `?` and the client sees JSON `null` — the protocol's
    /// "nothing here".
    fn handle<R>(
        &mut self,
        req: Request,
        f: impl FnOnce(&mut Self, R::Params) -> R::Result,
    ) -> Option<Response>
    where
        R: lsp_types::request::Request,
    {
        let (id, params) = match req.extract::<R::Params>(R::METHOD) {
            Ok(pair) => pair,
            Err(ExtractError::JsonError { method, error }) => {
                log(&format!("bad params for {method}: {error}"));
                return None;
            }
            Err(ExtractError::MethodMismatch(req)) => {
                log(&format!("method mismatch on {}", req.method));
                return None;
            }
        };

        let result = f(self, params);
        match serde_json::to_value(result) {
            Ok(value) => Some(Response::new_ok(id, value)),
            Err(e) => Some(Response::new_err(
                id,
                lsp_server::ErrorCode::InternalError as i32,
                e.to_string(),
            )),
        }
    }

    /// The document and byte offset a position refers to.
    fn locate(
        &self,
        pos: &TextDocumentPositionParams,
    ) -> Option<(&crate::documents::Document, u32)> {
        let doc = self.docs.get(&pos.text_document.uri)?;
        let offset = doc.line_index.offset(&doc.text, pos.position, self.enc);
        Some((doc, offset))
    }

    // ---- notifications -----------------------------------------------

    fn notification(&mut self, note: Notification) {
        match note.method.as_str() {
            notification::DidOpenTextDocument::METHOD => {
                if let Some(p) = extract::<notification::DidOpenTextDocument>(note) {
                    let d = p.text_document;
                    self.docs.open(d.uri.clone(), d.text, d.version);
                    self.reindex(&d.uri);
                    self.publish(&d.uri);
                }
            }
            notification::DidChangeTextDocument::METHOD => {
                if let Some(p) = extract::<notification::DidChangeTextDocument>(note) {
                    let uri = p.text_document.uri;
                    let enc = self.enc;
                    if let Some(doc) = self.docs.get_mut(&uri) {
                        doc.apply(p.text_document.version, &p.content_changes, enc);
                    }
                    self.reindex(&uri);
                    self.publish(&uri);
                }
            }
            notification::DidCloseTextDocument::METHOD => {
                if let Some(p) = extract::<notification::DidCloseTextDocument>(note) {
                    let uri = p.text_document.uri;
                    self.docs.close(&uri);
                    // Fall back to what is on disk, so closing a buffer does
                    // not remove its class from the index.
                    let path = uri_to_path(&uri);
                    match std::fs::read_to_string(&path) {
                        Ok(text) => self.index.index_file(&path, &text),
                        Err(_) => self.index.remove_file(&path),
                    }
                    // Clear the squiggles: a closed file has no diagnostics.
                    self.send_diagnostics(uri, Vec::new(), None);
                }
            }
            notification::DidSaveTextDocument::METHOD => {
                if let Some(p) = extract::<notification::DidSaveTextDocument>(note) {
                    self.reindex(&p.text_document.uri);
                }
            }
            _ => {}
        }
    }

    /// Re-index one file from its buffer. Cheap: one file, one parse.
    fn reindex(&mut self, uri: &Url) {
        if let Some(doc) = self.docs.get(uri) {
            self.index.index_file(&uri_to_path(uri), &doc.text);
        }
    }

    fn publish(&self, uri: &Url) {
        let Some(doc) = self.docs.get(uri) else {
            return;
        };
        let diagnostics = features::diagnostics::diagnostics(doc, self.enc);
        self.send_diagnostics(uri.clone(), diagnostics, Some(doc.version));
    }

    fn send_diagnostics(&self, uri: Url, diagnostics: Vec<Diagnostic>, version: Option<i32>) {
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        };
        self.send(Message::Notification(Notification::new(
            notification::PublishDiagnostics::METHOD.to_string(),
            params,
        )));
    }

    fn send(&self, msg: impl Into<Message>) {
        // A send failure means the editor is gone; the loop will see the
        // closed channel and exit on its own.
        let _ = self.sender.send(msg.into());
    }
}

fn extract<N: lsp_types::notification::Notification>(note: Notification) -> Option<N::Params> {
    match note.extract::<N::Params>(N::METHOD) {
        Ok(params) => Some(params),
        Err(e) => {
            log(&format!("bad params for {}: {e:?}", N::METHOD));
            None
        }
    }
}

/// Diagnostics and status go to stderr. stdout is the protocol channel, and
/// writing anything else to it corrupts the stream — the exact failure that
/// pushed `LanguageServer.quark` onto UDP.
fn log(message: &str) {
    eprintln!("[sclang-lsp] {message}");
}

/// Prefer UTF-8 when the client offers it, since it makes position conversion
/// free. UTF-16 is the protocol default and the guaranteed fallback.
fn negotiate_encoding(params: &InitializeParams) -> PositionEncoding {
    let offered = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref());

    match offered {
        Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => PositionEncoding::Utf8,
        Some(kinds) if kinds.contains(&PositionEncodingKind::UTF32) => PositionEncoding::Utf32,
        _ => PositionEncoding::Utf16,
    }
}

/// Where to look for code: the stock class library plus whatever the editor
/// opened. Both are indexed the same way — there is no privileged source.
fn roots_from(params: &InitializeParams) -> Vec<PathBuf> {
    // An explicit setting replaces the guessed locations rather than adding to
    // them, so a user pointing at one SuperCollider build does not silently
    // get a second one's class library merged in.
    let configured = params
        .initialization_options
        .as_ref()
        .and_then(|v| v.get("classLibraryPaths"))
        .and_then(|v| v.as_array())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|p| p.as_str())
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        });

    let mut roots = configured.unwrap_or_else(default_roots);

    #[allow(deprecated)] // `root_uri` predates workspace folders but is still sent.
    let folders = params
        .workspace_folders
        .as_ref()
        .map(|f| f.iter().map(|f| f.uri.clone()).collect::<Vec<_>>())
        .or_else(|| params.root_uri.clone().map(|u| vec![u]))
        .unwrap_or_default();

    for uri in folders {
        if let Ok(path) = uri.to_file_path() {
            roots.push(path);
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

fn initialize_result(enc: PositionEncoding) -> InitializeResult {
    InitializeResult {
        capabilities: capabilities(enc),
        server_info: Some(ServerInfo {
            name: "sclang-lsp".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    }
}

pub fn capabilities(enc: PositionEncoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(match enc {
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
            PositionEncoding::Utf32 => PositionEncodingKind::UTF32,
        }),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                ..Default::default()
            },
        )),
        completion_provider: Some(CompletionOptions {
            // `.` is the only character that changes what completion means.
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        signature_help_provider: Some(SignatureHelpOptions {
            // `(` opens a call; `,` moves to the next parameter.
            trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
            retrigger_characters: Some(vec![",".to_string()]),
            work_done_progress_options: Default::default(),
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        ..Default::default()
    }
}
