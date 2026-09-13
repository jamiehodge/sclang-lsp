//! Build a corpus of SuperCollider *script* code, for differential testing.
//!
//! Every other oracle here runs over the class library, which is `.sc` class
//! files. Those never contain a top-level `( … )` block, a bare `var`, or any
//! of the other syntax `cmdlinecode` allows — so the syntax users actually type
//! into a scratch buffer had no differential coverage at all. Two bugs found by
//! hand lived in exactly that gap.
//!
//! The help files are the best source of it: thousands of runnable examples,
//! written by the people who designed the language. Each `code::` block becomes
//! one snippet, alongside any real `.scd` files found.
//!
//!     cargo run --release --example scd_corpus -- <outdir> <dir>...

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: scd_corpus <outdir> <dir>...");
        std::process::exit(2);
    };

    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("create the corpus directory");

    let mut index = String::new();
    let mut n = 0usize;
    let (mut blocks, mut scripts) = (0usize, 0usize);

    for root in args {
        for file in walk(Path::new(&root)) {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let snippets: Vec<String> = match file.extension().and_then(|e| e.to_str()) {
                Some("schelp") => code_blocks(&text),
                Some("scd") => vec![text],
                _ => continue,
            };

            for snippet in snippets {
                // A snippet of only whitespace or comments tells us nothing and
                // both sides trivially accept it.
                if snippet.trim().is_empty() {
                    continue;
                }
                n += 1;
                let name = format!("{n:05}.scd");
                fs::write(out.join(&name), &snippet).expect("write a snippet");
                index.push_str(&format!("{name}\t{}\n", file.display()));
            }

            match file.extension().and_then(|e| e.to_str()) {
                Some("schelp") => blocks += 1,
                _ => scripts += 1,
            }
        }
    }

    fs::write(out.join("index.tsv"), index).expect("write the index");
    println!(
        "corpus: {n} snippets from {blocks} help files and {scripts} scripts -> {}",
        out.display()
    );
}

/// The `code::` … `::` blocks of a help file.
///
/// Only the block form, where `code::` is alone on its line. The inline form,
/// `code::foo::`, is a fragment rather than something that has to parse.
fn code_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        match &mut current {
            None if trimmed == "code::" => current = Some(String::new()),
            None => {}
            Some(_) if trimmed == "::" => {
                out.push(current.take().unwrap());
            }
            Some(body) => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }

    out
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("schelp") | Some("scd")
            ) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}
