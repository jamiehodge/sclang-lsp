//! Corpus-scale stability sweep: prefixes and mutations over real source.
//!
//! The unit tests in `tests/idempotence.rs` assert the same properties over a
//! handful of samples so they stay fast. This runs them over the whole class
//! library, which is where they have teeth — real files are long, deeply
//! nested, and contain constructs nobody would think to write by hand.
//!
//! Any panic is caught and reported rather than aborting the sweep, so one bad
//! file does not hide the rest.
//!
//! ```text
//! cargo run --release --example stability -- <dir>... [--prefix-step N]
//! ```

use sclang_syntax::{parse, Child, SyntaxNode};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn print_tree(node: &SyntaxNode, source: &str, out: &mut String) {
    for child in &node.children {
        match child {
            Child::Node(n) => print_tree(n, source, out),
            Child::Token(t) => out.push_str(t.text(source)),
        }
    }
}

/// Parse and reprint, returning whether every byte survived.
fn lossless(src: &str) -> bool {
    let p = parse(src);
    let mut out = String::with_capacity(src.len());
    print_tree(&p.root, src, &mut out);
    out == src
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("sc") | Some("scd")
        ) {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dirs = Vec::new();
    let mut step = 7usize; // prefix sampling stride
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--prefix-step" {
            step = it.next().and_then(|s| s.parse().ok()).unwrap_or(7);
        } else {
            dirs.push(a.clone());
        }
    }
    if dirs.is_empty() {
        eprintln!("usage: stability <dir>... [--prefix-step N]");
        std::process::exit(2);
    }

    let mut files = Vec::new();
    for d in &dirs {
        collect(Path::new(d), &mut files);
    }
    files.sort();

    let mut rng = Rng(0xf00d_d00d_1234_5678);
    let (mut prefixes, mut mutations) = (0usize, 0usize);
    let mut panics: Vec<String> = Vec::new();
    let mut lossy: Vec<String> = Vec::new();

    let started = std::time::Instant::now();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        // Every Nth prefix: what the parser sees while the file is typed.
        let mut end = 0usize;
        while end <= src.len() {
            if src.is_char_boundary(end) {
                let prefix = &src[..end];
                match catch_unwind(AssertUnwindSafe(|| lossless(prefix))) {
                    Ok(true) => {}
                    Ok(false) => lossy.push(format!("{name} prefix len {end}")),
                    Err(_) => panics.push(format!("{name} prefix len {end}")),
                }
                prefixes += 1;
            }
            end += step;
        }

        // Random single-character deletions and delimiter insertions.
        let chars = [
            '{', '}', '(', ')', '[', ']', ';', ',', '"', '\'', '\\', '^', '|', '#',
        ];
        for _ in 0..40 {
            let mut i = rng.below(src.len().max(1));
            while i < src.len() && !src.is_char_boundary(i) {
                i -= 1;
            }
            let mutated = if rng.next().is_multiple_of(2) {
                let mut m = String::with_capacity(src.len());
                m.push_str(&src[..i]);
                let mut rest = src[i..].chars();
                rest.next();
                m.push_str(rest.as_str());
                m
            } else {
                let c = chars[rng.below(chars.len())];
                let mut m = String::with_capacity(src.len() + 1);
                m.push_str(&src[..i]);
                m.push(c);
                m.push_str(&src[i..]);
                m
            };
            match catch_unwind(AssertUnwindSafe(|| lossless(&mutated))) {
                Ok(true) => {}
                Ok(false) => lossy.push(format!("{name} mutation at {i}")),
                Err(_) => panics.push(format!("{name} mutation at {i}")),
            }
            mutations += 1;
        }
    }

    println!("files              : {}", files.len());
    println!("prefixes parsed    : {prefixes}");
    println!("mutations parsed   : {mutations}");
    println!("panics             : {}", panics.len());
    println!("losslessness breaks: {}", lossy.len());
    println!(
        "elapsed            : {:.1}s",
        started.elapsed().as_secs_f64()
    );

    for p in panics.iter().take(10) {
        println!("  PANIC  {p}");
    }
    for l in lossy.iter().take(10) {
        println!("  LOSSY  {l}");
    }
    if !panics.is_empty() || !lossy.is_empty() {
        std::process::exit(1);
    }
}
