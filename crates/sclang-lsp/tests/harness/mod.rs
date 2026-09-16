//! Drives a real server over an in-memory transport.
//!
//! These tests speak the protocol rather than calling the feature functions
//! directly, so they cover the parts most likely to be wrong in practice:
//! capability negotiation, parameter shapes, and the order of the handshake.

use crossbeam_channel::RecvTimeoutError;
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::*;
use std::path::PathBuf;
use std::time::Duration;

/// Long enough to absorb a loaded machine, short enough that a deadlock fails
/// the test instead of hanging it.
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Harness {
    client: Connection,
    thread: Option<std::thread::JoinHandle<()>>,
    next_id: i32,
    pub dir: PathBuf,
    /// Requests the *server* sent us, kept because they arrive during the
    /// handshake and would otherwise be discarded before a test could look.
    seen_requests: std::cell::RefCell<Vec<Request>>,
    /// What the server advertised at `initialize`. A client keeps these for
    /// the life of the session, so a test that decodes a response should use
    /// them rather than anything the crate knows privately.
    pub capabilities: ServerCapabilities,
}

impl Harness {
    /// Start a server whose entire class library is a temp directory of the
    /// given files.
    pub fn start(name: &str, files: &[(&str, &str)]) -> Harness {
        let dir = std::env::temp_dir().join(format!(
            "sclang-lsp-test-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, source) in files {
            std::fs::write(dir.join(file), source).unwrap();
        }

        let (server_conn, client) = Connection::memory();
        let thread = std::thread::spawn(move || {
            sclang_lsp::server::Server::run(server_conn).unwrap();
        });

        let mut h = Harness {
            client,
            thread: Some(thread),
            next_id: 0,
            dir,
            seen_requests: std::cell::RefCell::new(Vec::new()),
            capabilities: ServerCapabilities::default(),
        };
        h.capabilities = h.initialize().capabilities;
        h
    }

    fn initialize(&mut self) -> InitializeResult {
        let params = serde_json::json!({
            "capabilities": {
                // The server registers a file watch only if told the client
                // can honour one, so this has to be here for that path to run
                // at all.
                "workspace": {
                    "didChangeWatchedFiles": { "dynamicRegistration": true },
                },
            },
            "initializationOptions": {
                "classLibraryPaths": [self.dir.to_str().unwrap()],
            },
        });
        let id = self.send_request("initialize", params);
        let response = self.await_response(id);
        assert!(response.error.is_none(), "initialize failed: {response:?}");
        let result: InitializeResult = serde_json::from_value(response.result.unwrap()).unwrap();

        self.send_notification("initialized", serde_json::json!({}));

        // The class library scan runs in the background; wait for the server
        // to say it finished, or every assertion below would race it.
        self.await_notification("window/logMessage");
        result
    }

    pub fn open(&mut self, name: &str, text: &str) -> Url {
        let uri = Url::from_file_path(self.dir.join(name)).unwrap();
        self.send_notification(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": uri, "languageId": "supercollider",
                    "version": 1, "text": text,
                }
            }),
        );
        uri
    }

    pub fn change(&mut self, uri: &Url, version: i32, range: Range, text: &str) {
        self.send_notification(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "range": range, "text": text }],
            }),
        );
    }

    pub fn close(&mut self, uri: &Url) {
        self.send_notification(
            "textDocument/didClose",
            serde_json::json!({ "textDocument": { "uri": uri } }),
        );
    }

    /// Send a request whose params carry a text document position.
    pub fn request_at<R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        uri: &Url,
        line: u32,
        character: u32,
    ) -> R {
        self.request(
            method,
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        )
    }

    pub fn request<R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> R {
        let id = self.send_request(method, params);
        let response = self.await_response(id);
        assert!(response.error.is_none(), "{method} failed: {response:?}");
        serde_json::from_value(response.result.unwrap()).unwrap()
    }

    pub fn send_request(&mut self, method: &str, params: serde_json::Value) -> RequestId {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.client
            .sender
            .send(Message::Request(Request {
                id: id.clone(),
                method: method.to_string(),
                params,
            }))
            .unwrap();
        id
    }

    pub fn send_notification(&self, method: &str, params: serde_json::Value) {
        self.client
            .sender
            .send(Message::Notification(Notification {
                method: method.to_string(),
                params,
            }))
            .unwrap();
    }

    pub fn await_response(&self, id: RequestId) -> Response {
        loop {
            if let Message::Response(r) = self.recv() {
                if r.id == id {
                    return r;
                }
            }
        }
    }

    /// Wait for the next notification with this method, ignoring others.
    pub fn await_notification(&self, method: &str) -> serde_json::Value {
        loop {
            if let Message::Notification(n) = self.recv() {
                if n.method == method {
                    return n.params;
                }
            }
        }
    }

    /// Diagnostics for a specific document, skipping any for other files.
    pub fn await_diagnostics(&self, uri: &Url) -> PublishDiagnosticsParams {
        loop {
            let params = self.await_notification("textDocument/publishDiagnostics");
            let params: PublishDiagnosticsParams = serde_json::from_value(params).unwrap();
            if &params.uri == uri {
                return params;
            }
        }
    }

    /// Wait for a request the *server* makes of us.
    ///
    /// It may already have arrived: the handshake reads messages until the
    /// scan reports in, and with a small class library that can happen before
    /// the server has processed `initialized`. So check what was seen, then
    /// keep reading.
    pub fn await_server_request(&self, method: &str) -> serde_json::Value {
        if let Some(params) = self
            .seen_requests
            .borrow()
            .iter()
            .find(|r| r.method == method)
            .map(|r| r.params.clone())
        {
            return params;
        }
        loop {
            if let Message::Request(request) = self.recv() {
                if request.method == method {
                    return request.params;
                }
            }
        }
    }

    fn recv(&self) -> Message {
        match self.client.receiver.recv_timeout(TIMEOUT) {
            Ok(Message::Request(request)) => {
                self.seen_requests.borrow_mut().push(request.clone());
                Message::Request(request)
            }
            Ok(msg) => msg,
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for the server"),
            Err(RecvTimeoutError::Disconnected) => panic!("server disconnected"),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // A clean shutdown proves the lifecycle works; it also stops the
        // server thread leaking into the next test.
        let id = self.next_id + 1;
        let _ = self.client.sender.send(Message::Request(Request {
            id: RequestId::from(id),
            method: "shutdown".to_string(),
            params: serde_json::Value::Null,
        }));
        let _ = self.client.sender.send(Message::Notification(Notification {
            method: "exit".to_string(),
            params: serde_json::Value::Null,
        }));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A library with a superclass worth inheriting from: slots that subclasses
/// read, and a method that dispatches back through `this`.
///
/// Deliberately several files. The point of both is that the answer is written
/// in a class the buffer being edited does not contain.
pub fn animal_library() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Object.sc", "Object {\n\tpostln { ^this }\n}\n"),
        (
            "Animal.sc",
            "Animal : Object {\n\
             \tvar <>name, legs;\n\
             \tclassvar <>census;\n\
             \tspeak { ^this.sound }\n\
             \tsound { ^\"...\" }\n\
             }\n",
        ),
        ("Cat.sc", "Cat : Animal {\n\tsound { ^\"meow\" }\n}\n"),
    ]
}

/// A miniature class library: enough shape to exercise inheritance without
/// depending on a SuperCollider install.
pub fn mini_library() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Object.sc",
            "Object {\n\tpostln { ^this }\n\tdump { ^this }\n}\n",
        ),
        (
            "UGen.sc",
            "UGen : Object {\n\t// Base class for unit generators.\n\t*multiNew { |...args| ^args }\n}\n",
        ),
        (
            // The class comment sits above the class, the method comment above
            // the method — which is what decides who each one documents.
            "SinOsc.sc",
            "// A sine oscillator.\nSinOsc : UGen {\n\t// Audio rate.\n\t*ar { |freq = 440, phase = 0, mul = 1, add = 0|\n\t\t^this.multiNew(freq, phase)\n\t}\n\t*kr { |freq = 440| ^this.multiNew(freq) }\n}\n",
        ),
        (
            "Saw.sc",
            "Saw : UGen {\n\t*ar { |freq = 440| ^this.multiNew(freq) }\n}\n",
        ),
    ]
}
