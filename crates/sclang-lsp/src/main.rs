//! `sclang-lsp` — a language server for SuperCollider.
//!
//! Speaks LSP over stdio. The server owns the connection with the editor and
//! parses SuperCollider itself, so it needs no sclang to answer; see
//! ARCHITECTURE.md for why that inversion is the whole design.

use lsp_server::Connection;

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    if std::env::args().any(|a| a == "--version") {
        println!("sclang-lsp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "sclang-lsp {}\n\n\
             A language server for SuperCollider. Speaks LSP over stdio, so it\n\
             is started by an editor rather than run by hand.\n\n\
             Options:\n  \
               --version   print the version and exit\n  \
               --help      print this message\n",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let (connection, io_threads) = Connection::stdio();
    sclang_lsp::server::Server::run(connection)?;
    // Waits for the writer to drain, so the final response is not lost when
    // the process exits.
    io_threads.join()?;
    Ok(())
}
