//! Lex a tree of `.sc` / `.scd` files and report conformance.
//!
//! This is the check that matters for the port: real SuperCollider source, in
//! bulk, asserting that the lexer never panics, that the token stream tiles
//! every file exactly, and that the share of unclassifiable input is
//! essentially zero.
//!
//! ```text
//! cargo run --release --example conformance -- <dir> [<dir>...]
//! ```

use sclang_syntax::{parse, tokenize, Child, SyntaxKind, SyntaxNode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Reconstruct source text from a tree, to check losslessness.
fn reconstruct(node: &SyntaxNode, source: &str, out: &mut std::string::String) {
    for child in &node.children {
        match child {
            Child::Node(n) => reconstruct(n, source, out),
            Child::Token(t) => out.push_str(t.text(source)),
        }
    }
}

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

fn main() {
    let args: Vec<std::string::String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: conformance <dir> [<dir>...]");
        std::process::exit(2);
    }

    let mut files = Vec::new();
    for arg in &args {
        collect(Path::new(arg), &mut files);
    }
    files.sort();

    let mut total_tokens = 0usize;
    let mut total_bytes = 0usize;
    let mut error_tokens = 0usize;
    let mut files_with_errors: Vec<(PathBuf, usize)> = Vec::new();
    let mut not_lossless: Vec<PathBuf> = Vec::new();
    let mut by_kind: BTreeMap<std::string::String, usize> = BTreeMap::new();
    let mut tree_not_lossless: Vec<PathBuf> = Vec::new();
    let mut files_parsed_clean = 0usize;
    let mut parse_errors = 0usize;
    let mut files_with_parse_errors: Vec<(PathBuf, usize)> = Vec::new();
    let mut classes_found = 0usize;
    let mut methods_found = 0usize;
    let mut error_messages: BTreeMap<std::string::String, usize> = BTreeMap::new();
    let mut first_error_context: Vec<std::string::String> = Vec::new();

    let started = std::time::Instant::now();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let tokens = tokenize(&src);

        // Losslessness: tokens must tile the file with no gaps or overlaps.
        let mut offset = 0u32;
        let mut ok = true;
        for t in &tokens {
            if t.start != offset {
                ok = false;
                break;
            }
            offset = t.end;
        }
        if !ok || offset as usize != src.len() {
            not_lossless.push(path.clone());
        }

        let errs = tokens
            .iter()
            .filter(|t| t.kind == SyntaxKind::Error)
            .count();
        if errs > 0 {
            files_with_errors.push((path.clone(), errs));
        }
        for t in &tokens {
            *by_kind.entry(format!("{:?}", t.kind)).or_default() += 1;
        }
        // ---- parse ----
        let parsed = parse(&src);
        let mut rebuilt = std::string::String::new();
        reconstruct(&parsed.root, &src, &mut rebuilt);
        if rebuilt != src {
            tree_not_lossless.push(path.clone());
        }
        if parsed.errors.is_empty() {
            files_parsed_clean += 1;
        } else {
            parse_errors += parsed.errors.len();
            files_with_parse_errors.push((path.clone(), parsed.errors.len()));
            let first = &parsed.errors[0];
            // Normalise the token name out of the message so like group with like.
            let key = first
                .message
                .split(", found")
                .next()
                .unwrap_or("")
                .to_string();
            *error_messages.entry(key).or_default() += 1;
            if first_error_context.len() < 40 {
                let line_no = src[..first.start as usize].matches('\n').count() + 1;
                let line = src.lines().nth(line_no - 1).unwrap_or("").trim();
                first_error_context.push(format!(
                    "{}:{}  {}  |  {}",
                    path.file_name().unwrap().to_string_lossy(),
                    line_no,
                    first.message,
                    &line[..line.len().min(60)]
                ));
            }
        }
        for n in parsed.root.descendants() {
            match n.kind {
                SyntaxKind::ClassDef | SyntaxKind::ClassExtension => classes_found += 1,
                SyntaxKind::MethodDef => methods_found += 1,
                _ => {}
            }
        }

        total_tokens += tokens.len();
        error_tokens += errs;
        total_bytes += src.len();
    }
    let elapsed = started.elapsed();

    println!("files            : {}", files.len());
    println!("bytes            : {total_bytes}");
    println!("tokens           : {total_tokens}");
    println!(
        "error tokens     : {error_tokens} ({:.4}%)",
        100.0 * error_tokens as f64 / total_tokens.max(1) as f64
    );
    println!(
        "files w/ errors  : {} ({:.1}%)",
        files_with_errors.len(),
        100.0 * files_with_errors.len() as f64 / files.len().max(1) as f64
    );
    println!(
        "lossless (tokens): {}",
        if not_lossless.is_empty() {
            "ALL FILES"
        } else {
            "FAILED"
        }
    );
    println!("---- parser ----");
    println!(
        "files parsed clean: {files_parsed_clean} / {} ({:.2}%)",
        files.len(),
        100.0 * files_parsed_clean as f64 / files.len().max(1) as f64
    );
    println!("parse errors     : {parse_errors}");
    println!("classes found    : {classes_found}");
    println!("methods found    : {methods_found}");
    println!(
        "lossless (tree)  : {}",
        if tree_not_lossless.is_empty() {
            "ALL FILES"
        } else {
            "FAILED"
        }
    );
    if !files_with_parse_errors.is_empty() {
        files_with_parse_errors.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\ntop files by parse error:");
        for (p, n) in files_with_parse_errors.iter().take(8) {
            println!("  {n:>5}  {}", p.display());
        }
    }
    if !error_messages.is_empty() {
        let mut msgs: Vec<_> = error_messages.into_iter().collect();
        msgs.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\nmost common errors (first per file):");
        for (m, n) in msgs.iter().take(15) {
            println!("  {n:>5}  {m}");
        }
    }
    if !first_error_context.is_empty() {
        println!("\nsample sites:");
        for line in first_error_context.iter().take(12) {
            println!("  {line}");
        }
    }
    println!(
        "throughput       : {:.1} MB/s ({:.0} ms total)",
        total_bytes as f64 / 1e6 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0
    );

    if !not_lossless.is_empty() {
        println!("\nNOT LOSSLESS:");
        for p in not_lossless.iter().take(20) {
            println!("  {}", p.display());
        }
    }

    if !files_with_errors.is_empty() {
        files_with_errors.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\ntop files by error tokens:");
        for (p, n) in files_with_errors.iter().take(15) {
            println!("  {n:>5}  {}", p.display());
        }
    }

    println!("\ntoken kinds:");
    let mut kinds: Vec<_> = by_kind.into_iter().collect();
    kinds.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (k, n) in kinds.iter().take(18) {
        println!("  {n:>8}  {k}");
    }
}
