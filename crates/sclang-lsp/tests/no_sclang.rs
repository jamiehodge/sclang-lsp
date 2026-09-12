//! The test ARCHITECTURE.md asks for: "Delete sclang, start the server, and
//! confirm completion and goto-definition still work on the class library."
//!
//! It runs the real binary, over real stdio, in an environment with nothing on
//! `PATH` — so no `sclang` can be found or launched even if something later
//! tries. The design says nothing here may contact a running image, so if this
//! ever fails something has grown a dependency that is not supposed to exist.
//!
//! Running the shipped binary also covers what the in-process tests cannot:
//! that stdio framing is correct, and that nothing in the server writes stray
//! output to stdout — the failure that pushed `LanguageServer.quark` onto UDP.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Speaks LSP framing to a child process over pipes.
struct StdioClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl StdioClient {
    fn start(class_library: &Path) -> StdioClient {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sclang-lsp"))
            // Nothing on PATH: `sclang` is unreachable by name.
            .env_clear()
            .env("PATH", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start sclang-lsp");

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = StdioClient {
            child,
            stdin,
            stdout,
            next_id: 0,
        };

        let id = client.request(
            "initialize",
            serde_json::json!({
                "capabilities": {},
                "initializationOptions": {
                    "classLibraryPaths": [class_library.to_str().unwrap()],
                },
            }),
        );
        let response = client.await_id(id);
        assert!(
            response.get("error").is_none(),
            "initialize failed: {response}"
        );

        client.notify("initialized", serde_json::json!({}));
        // Wait for the background scan to report in.
        client.await_method("window/logMessage");
        client
    }

    fn send(&mut self, value: serde_json::Value) {
        let body = serde_json::to_string(&value).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }));
        id
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.send(serde_json::json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        }));
    }

    /// Read one framed message.
    fn recv(&mut self) -> serde_json::Value {
        let mut length = None;
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read failed");
            assert_ne!(read, 0, "server closed the connection");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = Some(value.parse::<usize>().unwrap());
            }
        }
        let length = length.expect("no Content-Length header");
        let mut body = vec![0u8; length];
        self.stdout.read_exact(&mut body).expect("short body");
        serde_json::from_slice(&body).expect("invalid JSON body")
    }

    fn await_id(&mut self, id: i64) -> serde_json::Value {
        loop {
            let msg = self.recv();
            if msg.get("id").and_then(|v| v.as_i64()) == Some(id) && msg.get("method").is_none() {
                return msg;
            }
        }
    }

    fn await_method(&mut self, method: &str) -> serde_json::Value {
        loop {
            let msg = self.recv();
            if msg.get("method").and_then(|v| v.as_str()) == Some(method) {
                return msg;
            }
        }
    }

    fn result(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.request(method, params);
        let response = self.await_id(id);
        assert!(
            response.get("error").is_none(),
            "{method} failed: {response}"
        );
        response
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.request("shutdown", serde_json::Value::Null);
        self.notify("exit", serde_json::Value::Null);
        let _ = self.child.wait();
    }
}

/// A class library of its own for each test.
///
/// Naming this by pid alone is not enough: cargo runs the tests in one process
/// on separate threads, so a shared directory means one test's cleanup deletes
/// the library another is still serving from.
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sclang-lsp-no-sclang-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Object.sc"), "Object {\n\tpostln { ^this }\n}\n").unwrap();
    std::fs::write(
        dir.join("UGen.sc"),
        "UGen : Object {\n\t*multiNew { |...args| ^args }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("SinOsc.sc"),
        "SinOsc : UGen {\n\t*ar { |freq = 440| ^this.multiNew(freq) }\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn completion_and_goto_work_with_no_sclang_on_the_path() {
    let dir = fixture("completion");
    let mut client = StdioClient::start(&dir);

    let uri = lsp_types::Url::from_file_path(dir.join("Test.scd")).unwrap();
    client.notify(
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": uri, "languageId": "supercollider",
                "version": 1, "text": "SinOsc.",
            }
        }),
    );

    // Completion, from the class library, with no image to ask.
    let completion = client.result(
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 7 },
        }),
    );
    let labels: Vec<&str> = completion["items"]
        .as_array()
        .expect("expected a completion list")
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"ar"), "{labels:?}");
    assert!(
        labels.contains(&"multiNew"),
        "inherited missing: {labels:?}"
    );

    // Goto definition, likewise.
    let definition = client.result(
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 2 },
        }),
    );
    let target = definition["uri"].as_str().expect("expected one location");
    assert!(target.ends_with("SinOsc.sc"), "{target}");

    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diagnostics_arrive_over_stdio() {
    let dir = fixture("diagnostics");
    let mut client = StdioClient::start(&dir);

    let uri = lsp_types::Url::from_file_path(dir.join("Broken.scd")).unwrap();
    client.notify(
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": uri, "languageId": "supercollider",
                "version": 1, "text": "SinOsc.",
            }
        }),
    );

    let note = client.await_method("textDocument/publishDiagnostics");
    let diagnostics = note["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["severity"], 1);

    drop(client);
    let _ = std::fs::remove_dir_all(&dir);
}
