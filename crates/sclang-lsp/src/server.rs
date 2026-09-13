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
use crate::references::ReferenceIndex;
use crate::workspace::{build_index, default_roots, IndexStats};

use crossbeam_channel::{Receiver, Sender};
use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::*;
use sclang_index::SymbolIndex;
use std::path::PathBuf;

/// Everything the request path reads.
pub struct Server {
    docs: DocumentStore,
    index: SymbolIndex,
    references: ReferenceIndex,
    /// What semantic tokens each open document was last sent, so a client can
    /// be told what changed rather than sent the whole array again.
    tokens: features::semantic_tokens::Cache,
    /// Whether the client will watch files for us if asked. There is no static
    /// capability for it, so the only way to find out is to read this and
    /// register afterwards.
    watches_files: bool,
    /// Ids for the requests this server makes. Only registration so far.
    next_request: i32,
    enc: PositionEncoding,
    sender: Sender<Message>,
}

/// What the background scan sends back when it finishes.
struct Scan {
    index: SymbolIndex,
    references: ReferenceIndex,
    stats: IndexStats,
}

impl Server {
    /// Run the server to completion over an already-connected transport.
    pub fn run(connection: Connection) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        let (id, params) = connection.initialize_start()?;
        let params: InitializeParams = serde_json::from_value(params)?;

        let enc = negotiate_encoding(&params);
        let roots = roots_from(&params);
        let watches_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|w| w.dynamic_registration)
            .unwrap_or(false);

        connection.initialize_finish(id, serde_json::to_value(initialize_result(enc))?)?;

        // Only now, with the editor unblocked, go and read the class library.
        let (scan_tx, scan_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let (index, references, stats) = build_index(&roots);
            let _ = scan_tx.send(Scan {
                index,
                references,
                stats,
            });
        });

        let mut server = Server {
            docs: DocumentStore::default(),
            index: SymbolIndex::default(),
            references: ReferenceIndex::default(),
            tokens: Default::default(),
            watches_files,
            next_request: 0,
            enc,
            sender: connection.sender.clone(),
        };
        // `initialize_finish` consumes the `initialized` notification, so the
        // event loop never sees it. This is the moment after the handshake,
        // which is the first point a server may ask the client for anything.
        server.watch_class_files();
        server.event_loop(&connection, scan_rx)
    }

    fn event_loop(
        &mut self,
        connection: &Connection,
        scan_rx: Receiver<Scan>,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        // The scan arrives once, and then its sender is dropped. A disconnected
        // channel is *always* ready, so leaving that branch in the select
        // afterwards makes `recv` return `Err` immediately, every time, and
        // the loop spins at 100% CPU for the life of the process. Swapping in
        // a channel that is never ready is what stops it.
        let never = crossbeam_channel::never::<Scan>();
        let mut scan_rx = Some(scan_rx);

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
                        // Responses to requests we made. The only one is the
                        // file-watch registration, whose failure would show up
                        // as stale answers rather than as anything to handle
                        // here.
                        Message::Response(_) => {}
                    }
                }
                recv(scan_rx.as_ref().unwrap_or(&never)) -> scan => {
                    // Whether the scan arrived or the thread went away, there
                    // is nothing more coming from here.
                    scan_rx = None;
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
        self.references = scan.references;
        for (uri, doc) in self.docs.iter() {
            let path = uri_to_path(uri);
            self.index.index_file(&path, &doc.text);
            self.references
                .index_parsed(&path, &doc.text, &doc.parse().root);
        }
        let s = scan.stats;
        let message = format!(
            "indexed {} files: {} classes, {} methods, {} occurrences{}",
            s.files,
            s.classes,
            s.methods,
            s.occurrences,
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
            request::InlayHintRequest::METHOD => {
                self.handle::<request::InlayHintRequest>(req, |s, p| {
                    let doc = s.docs.get(&p.text_document.uri)?;
                    Some(features::inlay_hints::inlay_hints(
                        doc, &s.index, p.range, s.enc,
                    ))
                })
            }
            request::DocumentHighlightRequest::METHOD => self
                .handle::<request::DocumentHighlightRequest>(req, |s, p| {
                    let uri = p.text_document_position_params.text_document.uri.clone();
                    let (doc, offset) = s.locate(&p.text_document_position_params)?;
                    features::document_highlight::document_highlight(
                        &uri,
                        doc,
                        &s.references,
                        offset,
                        s.enc,
                    )
                }),
            request::References::METHOD => self.handle::<request::References>(req, |s, p| {
                let uri = p.text_document_position.text_document.uri.clone();
                let (doc, offset) = s.locate(&p.text_document_position)?;
                let resolver = Resolver::new(&s.docs, s.enc);
                features::find_references::references(
                    &uri,
                    doc,
                    &s.references,
                    offset,
                    p.context.include_declaration,
                    &resolver,
                )
            }),
            request::PrepareRenameRequest::METHOD => self
                .handle_fallible::<request::PrepareRenameRequest>(req, |s, p| {
                    let (doc, offset) = s
                        .locate(&p)
                        .ok_or_else(|| "that document is not open".to_string())?;
                    features::rename::prepare_rename(doc, &s.index, offset, s.enc).map(Some)
                }),
            request::Rename::METHOD => self.handle_fallible::<request::Rename>(req, |s, p| {
                let uri = p.text_document_position.text_document.uri.clone();
                let (doc, offset) = s
                    .locate(&p.text_document_position)
                    .ok_or_else(|| "that document is not open".to_string())?;
                let resolver = Resolver::new(&s.docs, s.enc);
                features::rename::rename(
                    &uri,
                    doc,
                    &s.index,
                    &s.references,
                    offset,
                    &p.new_name,
                    &resolver,
                )
                .map(Some)
            }),
            request::HoverRequest::METHOD => self.handle::<request::HoverRequest>(req, |s, p| {
                let (doc, offset) = s.locate(&p.text_document_position_params)?;
                features::hover::hover(doc, &s.index, offset, s.enc)
            }),
            request::GotoDefinition::METHOD => {
                self.handle::<request::GotoDefinition>(req, |s, p| {
                    let uri = p.text_document_position_params.text_document.uri.clone();
                    let (doc, offset) = s.locate(&p.text_document_position_params)?;
                    let resolver = Resolver::new(&s.docs, s.enc);
                    features::goto::goto_definition(&uri, doc, &s.index, offset, &resolver)
                })
            }
            request::GotoImplementation::METHOD => {
                self.handle::<request::GotoImplementation>(req, |s, p| {
                    let (doc, offset) = s.locate(&p.text_document_position_params)?;
                    let resolver = Resolver::new(&s.docs, s.enc);
                    features::goto::goto_implementation(doc, &s.index, offset, &resolver)
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
            request::SemanticTokensFullRequest::METHOD => self
                .handle::<request::SemanticTokensFullRequest>(req, |s, p| {
                    let uri = p.text_document.uri;
                    let doc = s.docs.get(&uri)?;
                    let tokens = features::semantic_tokens::semantic_tokens(doc, s.enc);
                    Some(s.tokens.full(&uri, tokens).into())
                }),
            request::SemanticTokensFullDeltaRequest::METHOD => {
                self.handle::<request::SemanticTokensFullDeltaRequest>(req, |s, p| {
                    let uri = p.text_document.uri;
                    let doc = s.docs.get(&uri)?;
                    let tokens = features::semantic_tokens::semantic_tokens(doc, s.enc);
                    Some(s.tokens.delta(&uri, &p.previous_result_id, tokens))
                })
            }
            request::SemanticTokensRangeRequest::METHOD => self
                .handle::<request::SemanticTokensRangeRequest>(req, |s, p| {
                    let doc = s.docs.get(&p.text_document.uri)?;
                    let data =
                        features::semantic_tokens::semantic_tokens_range(doc, p.range, s.enc);
                    // No result id. A range covers part of a document, so it
                    // can never be the thing a later delta is measured
                    // against, and labelling it would invite exactly that.
                    Some(
                        SemanticTokens {
                            result_id: None,
                            data,
                        }
                        .into(),
                    )
                }),
            request::FoldingRangeRequest::METHOD => {
                self.handle::<request::FoldingRangeRequest>(req, |s, p| {
                    let doc = s.docs.get(&p.text_document.uri)?;
                    Some(features::folding_range::folding_ranges(doc))
                })
            }
            request::SelectionRangeRequest::METHOD => self
                .handle::<request::SelectionRangeRequest>(req, |s, p| {
                    let doc = s.docs.get(&p.text_document.uri)?;
                    Some(features::selection_range::selection_ranges(
                        doc,
                        &p.positions,
                        s.enc,
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
        let (id, params) = match extract_params::<R>(req) {
            Ok(pair) => pair,
            Err(response) => return response,
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

    /// Dispatch a request whose failure the user should be told about.
    ///
    /// Rename refuses more often than it succeeds, and *why* is the useful
    /// part — "this is a method and dispatch is dynamic" is guidance, where a
    /// silent null would look like a broken server. The protocol carries that
    /// as a request-failed error, which clients surface verbatim.
    fn handle_fallible<R>(
        &mut self,
        req: Request,
        f: impl FnOnce(&mut Self, R::Params) -> Result<R::Result, String>,
    ) -> Option<Response>
    where
        R: lsp_types::request::Request,
    {
        let (id, params) = match extract_params::<R>(req) {
            Ok(pair) => pair,
            Err(response) => return response,
        };

        match f(self, params) {
            Ok(result) => match serde_json::to_value(result) {
                Ok(value) => Some(Response::new_ok(id, value)),
                Err(e) => Some(Response::new_err(
                    id,
                    lsp_server::ErrorCode::InternalError as i32,
                    e.to_string(),
                )),
            },
            // -32803 is RequestFailed: the request was well formed and the
            // server declined it, which is exactly this case.
            Err(message) => Some(Response::new_err(id, -32803, message)),
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
                    self.tokens.forget(&uri);
                    // Fall back to what is on disk, so closing a buffer does
                    // not remove its class from the index.
                    let path = uri_to_path(&uri);
                    match std::fs::read_to_string(&path) {
                        Ok(text) => {
                            self.index.index_file(&path, &text);
                            self.references.index_file(&path, &text);
                        }
                        Err(_) => {
                            self.index.remove_file(&path);
                            self.references.remove_file(&path);
                        }
                    }
                    // Clear the squiggles: a closed file has no diagnostics.
                    self.send_diagnostics(uri, Vec::new(), None);
                }
            }
            notification::DidChangeWatchedFiles::METHOD => {
                if let Some(p) = extract::<notification::DidChangeWatchedFiles>(note) {
                    for change in p.changes {
                        self.disk_changed(&change.uri, change.typ);
                    }
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

    /// Ask the client to report `.sc` files changing on disk.
    ///
    /// Without this the index only ever hears about files the editor has open,
    /// so a `git checkout`, a quark install, or an edit made in another
    /// program leaves goto-definition pointing at locations that have moved.
    /// Nothing announces that; the answers are simply wrong, which is the
    /// worst way for an index to fail.
    ///
    /// `.sc` only, matching what the startup scan reads. A `.scd` is a script
    /// meant to be evaluated rather than a file of class definitions, and
    /// several in the stock library do not parse as a whole file by design.
    fn watch_class_files(&mut self) {
        if !self.watches_files {
            return;
        }
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.sc".to_string()),
                // Omitted means create, change and delete, which is all three.
                kind: None,
            }],
        };
        let params = RegistrationParams {
            registrations: vec![Registration {
                id: "sclang-lsp-watch-class-files".to_string(),
                method: notification::DidChangeWatchedFiles::METHOD.to_string(),
                register_options: serde_json::to_value(options).ok(),
            }],
        };

        self.next_request += 1;
        self.send(Message::Request(Request {
            id: RequestId::from(self.next_request),
            method: request::RegisterCapability::METHOD.to_string(),
            params: match serde_json::to_value(params) {
                Ok(value) => value,
                Err(e) => {
                    log(&format!("could not register a file watch: {e}"));
                    return;
                }
            },
        }));
    }

    /// Re-read one file the client says changed underneath us.
    ///
    /// An open buffer wins, always. The server owns document text, so what is
    /// on disk beneath an unsaved edit is the stale copy — and saving already
    /// re-indexes through `didSave`.
    fn disk_changed(&mut self, uri: &Url, change: FileChangeType) {
        if self.docs.get(uri).is_some() {
            return;
        }
        let path = uri_to_path(uri);
        if path.extension().is_none_or(|e| e != "sc") {
            return;
        }

        // A delete, or a create/change for something that cannot be read —
        // removed again between the event and this read, or not valid UTF-8.
        // Dropping what was recorded is right for all of them: the alternative
        // is keeping symbols for a file that no longer says what they claim.
        let source = (change != FileChangeType::DELETED)
            .then(|| std::fs::read_to_string(&path).ok())
            .flatten();

        match source {
            Some(text) => {
                self.index.index_file(&path, &text);
                self.references.index_file(&path, &text);
            }
            None => {
                self.index.remove_file(&path);
                self.references.remove_file(&path);
            }
        }
    }

    /// Re-index one file from its buffer. Cheap: one file, one parse.
    fn reindex(&mut self, uri: &Url) {
        if let Some(doc) = self.docs.get(uri) {
            let path = uri_to_path(uri);
            self.index.index_file(&path, &doc.text);
            // The buffer's tree is already built; parsing it again here would
            // double the cost of every keystroke.
            self.references
                .index_parsed(&path, &doc.text, &doc.parse().root);
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

/// Pull typed params out of a request, or produce the error response.
///
/// A request must always be answered. Logging the problem and returning
/// nothing leaves the client waiting on an id that will never come — which is
/// a hang, not a failure, and far harder to diagnose from the other end.
#[allow(clippy::type_complexity)]
fn extract_params<R>(req: Request) -> Result<(RequestId, R::Params), Option<Response>>
where
    R: lsp_types::request::Request,
{
    // Taken before `extract` consumes the request: without it the failure
    // path has no id to answer on, which is how this hung in the first place.
    let id = req.id.clone();

    match req.extract::<R::Params>(R::METHOD) {
        Ok(pair) => Ok(pair),
        Err(ExtractError::JsonError { method, error }) => {
            log(&format!("bad params for {method}: {error}"));
            Err(Some(Response::new_err(
                id,
                lsp_server::ErrorCode::InvalidParams as i32,
                format!("bad params for {method}: {error}"),
            )))
        }
        Err(ExtractError::MethodMismatch(req)) => {
            log(&format!("method mismatch on {}", req.method));
            Err(Some(Response::new_err(
                req.id,
                lsp_server::ErrorCode::InternalError as i32,
                format!("method mismatch on {}", req.method),
            )))
        }
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
        inlay_hint_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        // Every class defining a selector. In a dynamically dispatched
        // language this is the question with a real answer, where definition
        // has to pick one.
        implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
        references_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            // Asked before the box opens, so a refusal explains itself instead
            // of appearing after the user has typed a new name.
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        // Colour from the parse tree. Offered unconditionally: a client that
        // does not consume semantic tokens simply never asks, and one that
        // also has a grammar layers these over it rather than replacing it.
        semantic_tokens_provider: Some(
            SemanticTokensOptions {
                legend: features::semantic_tokens::legend(),
                // Deltas, because the payload is the cost here rather than
                // the computation: a long class file is a few hundred
                // kilobytes of JSON, and a keystroke changes a few tokens of
                // it.
                full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                // A client painting a long class-library file asks for the
                // viewport before it asks for the rest.
                range: Some(true),
                work_done_progress_options: Default::default(),
            }
            .into(),
        ),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        ..Default::default()
    }
}
