//! Corpus-scale indenter sweep.
//!
//! The unit tests in `tests/indent.rs` assert the same properties over a
//! handful of samples so they stay fast. This runs them over real source,
//! which is where they have teeth — the class library is long, deeply nested
//! and full of constructs nobody would think to write by hand, and the `.scd`
//! corpus is where the top-level `( … )` region idiom actually lives.
//!
//! Three properties, in the order they matter:
//!
//! - **content** — only leading whitespace changed;
//! - **fixed point** — indenting the result again changes nothing;
//! - **no panic** — a damaged tree still produces an answer.
//!
//! Any panic is caught and reported rather than aborting the sweep, so one bad
//! file does not hide the rest.
//!
//! The properties hold whatever the style, but *already indented* only means
//! anything when the style matches the corpus: pass `--tabs` for the class
//! library, which is tab-indented throughout.
//!
//! ```text
//! cargo run --release --example indent_sweep -- <dir>... [--tabs]
//! ```

use sclang_syntax::{parse_with, reindent, IndentStyle, Mode};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn indented(source: &str, mode: Mode, style: IndentStyle) -> String {
    reindent(source, &parse_with(source, mode), style)
}

/// Every line with its leading whitespace removed. Two sources agreeing on
/// this differ in indentation and in nothing else.
fn stripped(source: &str) -> Vec<&str> {
    source.lines().map(str::trim_start).collect()
}

/// The first property this file breaks, if it breaks one.
fn check(source: &str, mode: Mode, style: IndentStyle) -> Result<(), String> {
    let once = indented(source, mode, style);
    if stripped(&once) != stripped(source) {
        return Err("content changed".to_string());
    }
    if once.bytes().filter(|b| *b == b'\n').count()
        != source.bytes().filter(|b| *b == b'\n').count()
    {
        return Err("line count changed".to_string());
    }
    let twice = indented(&once, mode, style);
    if twice != once {
        let line = twice
            .lines()
            .zip(once.lines())
            .position(|(a, b)| a != b)
            .map(|n| n + 1)
            .unwrap_or(0);
        return Err(format!("not a fixed point, first at line {line}"));
    }
    Ok(())
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tabs = args.iter().any(|a| a == "--tabs");
    let dirs: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if dirs.is_empty() {
        eprintln!("usage: indent_sweep <dir>... [--tabs]");
        std::process::exit(2);
    }
    let style = IndentStyle {
        tab_size: 4,
        insert_spaces: !tabs,
    };

    let mut files = Vec::new();
    for dir in &dirs {
        collect(Path::new(dir), &mut files);
    }
    files.sort();

    let started = std::time::Instant::now();
    let mut checked = 0usize;
    let mut unreadable = 0usize;
    let mut already = 0usize;
    let mut panics: Vec<String> = Vec::new();
    let mut broken: Vec<String> = Vec::new();

    for path in &files {
        // Two class-library files are Latin-1 from the 2000s. Not this
        // sweep's problem; report them separately rather than as failures.
        let Ok(source) = std::fs::read_to_string(path) else {
            unreadable += 1;
            continue;
        };
        // A `.sc` file is a class file and anything else is interpreted, which
        // is the same rule the server applies to a URI.
        let mode = match path.extension().and_then(|e| e.to_str()) {
            Some("sc") => Mode::ClassFile,
            _ => Mode::Script,
        };
        let name = path.display().to_string();

        match catch_unwind(AssertUnwindSafe(|| check(&source, mode, style))) {
            Ok(Ok(())) => {
                if indented(&source, mode, style) == source {
                    already += 1;
                }
            }
            Ok(Err(why)) => broken.push(format!("{name}: {why}")),
            Err(_) => panics.push(name),
        }
        checked += 1;
    }

    println!(
        "style            : {}",
        if tabs { "tabs" } else { "4 spaces" }
    );
    println!("files            : {}", files.len());
    println!("checked          : {checked}");
    println!("unreadable       : {unreadable}");
    println!(
        "already indented : {already} ({:.1}%)",
        if checked == 0 {
            0.0
        } else {
            already as f64 * 100.0 / checked as f64
        }
    );
    println!("panics           : {}", panics.len());
    println!("property breaks  : {}", broken.len());
    println!("elapsed          : {:.1}s", started.elapsed().as_secs_f64());

    for p in panics.iter().take(10) {
        println!("  PANIC  {p}");
    }
    for b in broken.iter().take(10) {
        println!("  BREAK  {b}");
    }
    if !panics.is_empty() || !broken.is_empty() {
        std::process::exit(1);
    }
}
