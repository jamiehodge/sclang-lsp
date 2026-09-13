//! An idle server must use no CPU.
//!
//! This exists because of a bug it would have caught. The event loop selected
//! over the channel carrying the class library scan, and once that scan
//! arrived its sender was dropped — leaving a *disconnected* channel in the
//! select. A disconnected channel is always ready, so `recv` returned `Err`
//! immediately, forever, and the loop spun at 100% CPU for the life of the
//! process. Every request still answered correctly, so nothing else in the
//! suite noticed.
//!
//! Unix only: reading another process's CPU time portably is more machinery
//! than the check is worth, and the bug is not platform specific.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cumulative CPU seconds used by a process, from `ps`.
fn cpu_seconds(pid: u32) -> f64 {
    let out = Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    let text = String::from_utf8_lossy(&out.stdout);

    // `MM:SS.ss`, or `HH:MM:SS` once it has been running a while.
    text.trim().split(':').fold(0.0, |acc, part| {
        acc * 60.0 + part.parse::<f64>().unwrap_or(0.0)
    })
}

#[test]
fn an_idle_server_uses_no_cpu() {
    let dir = std::env::temp_dir().join(format!("sclang-lsp-idle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Object.sc"), "Object { foo { ^1 } }\n").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_sclang-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the server");

    let mut stdin = child.stdin.take().unwrap();

    // Drain stdout on a thread. An LSP body carries no trailing newline, so a
    // line-oriented read blocks forever once the server goes quiet — which is
    // precisely the state being tested.
    let seen = Arc::new(Mutex::new(String::new()));
    {
        let seen = Arc::clone(&seen);
        let mut stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    return;
                }
                seen.lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });
    }

    let send = |stdin: &mut std::process::ChildStdin, body: String| {
        write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        stdin.flush().unwrap();
    };

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "capabilities": {},
                "initializationOptions": { "classLibraryPaths": [dir.to_str().unwrap()] },
            },
        })
        .to_string(),
    );
    send(
        &mut stdin,
        serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }).to_string(),
    );

    // Wait for the server to say the scan finished, which is the moment the
    // channel disconnects and the old code began to spin.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut scanned = false;
    while !scanned && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        scanned = seen.lock().unwrap().contains("indexed");
    }
    assert!(scanned, "the server never reported finishing its scan");

    let pid = child.id();
    std::thread::sleep(Duration::from_millis(500));

    let before = cpu_seconds(pid);
    let window = Duration::from_secs(3);
    std::thread::sleep(window);
    let used = cpu_seconds(pid) - before;

    // A spinning loop burns a whole core: three seconds of wall clock means
    // about three seconds of CPU. An idle one should be indistinguishable from
    // zero, and the allowance here is for `ps` rounding rather than for work.
    let _ = child.kill();
    // Reaped rather than left as a zombie, which also makes the kill ordered
    // before the assertion below rather than racing it.
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        used < 0.2,
        "idle server used {used:.2}s of CPU over {}s of doing nothing",
        window.as_secs(),
    );
}
