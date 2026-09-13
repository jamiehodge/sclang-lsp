//! Corpus-scale check that semantic tokens encode cleanly.
//!
//! The delta encoding is the part that fails silently: five integers per
//! token, each relative to the one before it, so a wrong offset raises nothing
//! anywhere — it is colour sliding down the file from wherever it first went
//! wrong. The properties that catch it hold for *any* input, which is what
//! makes a sweep worth more here than more hand-written cases.
//!
//! The unit tests in `features::semantic_tokens` assert the same properties on
//! committed inputs so they run in CI. This runs them over whatever real
//! SuperCollider is on the machine, where they have teeth: the class library
//! for `.sc`, and `oracle/scd-corpus` — the `code::` blocks of the help files,
//! built by `oracle/run-scd.sh` — for the script syntax the class library
//! never contains.
//!
//! ```text
//! cargo run --release --example token_sweep -- <dir>...
//! ```
//!
//! Exits 2 if any file violates a property, naming the first one per file.

use lsp_types::Position;
use sclang_lsp::documents::Document;
use sclang_lsp::features::semantic_tokens::{legend, semantic_tokens};
use sclang_lsp::line_index::{LineIndex, PositionEncoding};
use sclang_syntax::Mode;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sc") | Some("scd")
        ) {
            out.push(path);
        }
    }
}

/// Check one file, returning the first property it breaks.
///
/// Both encodings, because the arithmetic differs and a file with a `°` or an
/// emoji in a comment only breaks one of them.
fn check(source: &str, mode: Mode) -> Result<usize, String> {
    let doc = Document::with_mode(source.to_string(), 1, mode);
    let legend = legend();
    let index = LineIndex::new(source);
    let mut counted = 0;

    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        let tokens = semantic_tokens(&doc, enc);
        counted = tokens.len();
        let (mut line, mut character) = (0, 0);
        let mut previous_end = 0;

        for (i, token) in tokens.iter().enumerate() {
            line += token.delta_line;
            character = if token.delta_line == 0 {
                character + token.delta_start
            } else {
                token.delta_start
            };
            let fail =
                |what: &str| Err(format!("token {i} at {line}:{character} ({enc:?}) {what}"));

            if token.length == 0 {
                return fail("has zero length");
            }
            if token.token_type as usize >= legend.token_types.len() {
                return fail("has a type that is not in the legend");
            }
            if token.token_modifiers_bitset >= (1 << legend.token_modifiers.len()) {
                return fail("has modifier bits that are not in the legend");
            }

            let start = Position::new(line, character);
            let end = Position::new(line, character + token.length);
            let from = index.offset(source, start, enc);
            let to = index.offset(source, end, enc);

            // `offset` clamps a position past the end of its line, so the
            // round trip is the only thing that notices one. Either end
            // landing short means the token claims text that is not there.
            if index.position(source, from, enc) != start {
                return fail("starts past the end of its line");
            }
            if index.position(source, to, enc) != end {
                return fail("runs past the end of its line");
            }
            if source[from as usize..to as usize].contains(['\n', '\r']) {
                return fail("spans a line break");
            }
            if from < previous_end {
                return fail("overlaps the token before it");
            }
            previous_end = to;
        }
    }

    Ok(counted)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: token_sweep <dir> [<dir>...]");
        std::process::exit(2);
    }

    let mut files = Vec::new();
    for arg in &args {
        collect(Path::new(arg), &mut files);
    }
    files.sort();

    let mut swept = 0usize;
    let mut tokens = 0usize;
    let mut unreadable = 0usize;
    let mut broken: Vec<(PathBuf, String)> = Vec::new();

    let started = std::time::Instant::now();
    for path in &files {
        // Two class-library files are Latin-1 from the 2000s. They are a
        // corpus problem rather than a token one.
        let Ok(source) = std::fs::read_to_string(path) else {
            unreadable += 1;
            continue;
        };
        // A `.sc` file is class definitions and everything else is
        // interpreted, which is the same split the server makes on open.
        let mode = if path.extension().and_then(|e| e.to_str()) == Some("sc") {
            Mode::ClassFile
        } else {
            Mode::Script
        };
        match check(&source, mode) {
            Ok(n) => tokens += n,
            Err(problem) => broken.push((path.clone(), problem)),
        }
        swept += 1;
    }

    println!("files swept       : {swept}");
    println!("tokens            : {tokens}");
    if unreadable > 0 {
        println!("not valid UTF-8   : {unreadable} (skipped)");
    }
    println!("elapsed           : {:.2?}", started.elapsed());

    if broken.is_empty() {
        println!("\nevery token ordered, on one line, and on the text it claims.");
        return;
    }

    println!("\n{} files broke a property:", broken.len());
    for (path, problem) in &broken {
        println!("  {}: {problem}", path.display());
    }
    std::process::exit(2);
}
