//! Diff script parsing against sclang's own compiler.
//!
//! `oracle/dump-scd.scd` asks sclang to `compile` every snippet in the corpus —
//! parse only, nothing runs — and records whether it succeeded. This parses the
//! same snippets and compares.
//!
//! The asymmetry matters. A snippet sclang accepts and this rejects is a bug
//! here, full stop. The other direction is softer: this parser is deliberately
//! error tolerant, and a fragment it makes sense of is not automatically wrong.
//! But a large gap that way means the grammar has drifted permissive, which is
//! how two adjacent blocks came to be read as a call.
//!
//!     ./oracle/run-scd.sh

use sclang_syntax::parse;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// How many disagreements to show before summarising.
const SHOW: usize = 15;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dump), Some(dir)) = (args.next(), args.next().map(PathBuf::from)) else {
        eprintln!("usage: scd_oracle <dump.tsv> <corpus dir>");
        std::process::exit(2);
    };

    // Where each snippet came from, so a failure names a real help file.
    let origins: HashMap<String, String> = fs::read_to_string(dir.join("index.tsv"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();

    let text = fs::read_to_string(&dump).unwrap_or_else(|e| {
        eprintln!("could not read {dump}: {e}");
        std::process::exit(2);
    });

    let mut compared = 0usize;
    let mut agreed = 0usize;
    let mut lossless = 0usize;
    let mut we_reject: Vec<(String, String)> = Vec::new();
    let mut we_accept: Vec<(String, String)> = Vec::new();
    // Grouped first errors: one cause usually accounts for most of a run.
    let mut reasons: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        let mut parts = line.split('\t');
        if parts.next() != Some("S") {
            continue;
        }
        let (Some(name), Some(verdict)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(source) = fs::read_to_string(dir.join(name)) else {
            continue;
        };

        let sclang_ok = verdict == "ok";
        let parsed = parse(&source);
        let ours_ok = parsed.is_ok();

        // Losslessness has to hold whatever the verdict: error recovery keeps
        // the rest of a broken snippet intact, which is the property an editor
        // depends on.
        if reconstruct(&parsed.root, &source) == source {
            lossless += 1;
        }

        compared += 1;
        let origin = origins.get(name).cloned().unwrap_or_default();
        match (sclang_ok, ours_ok) {
            (true, true) | (false, false) => agreed += 1,
            (true, false) => {
                let why = parsed
                    .errors
                    .first()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "(no error recorded)".to_string());
                *reasons.entry(why.clone()).or_insert(0) += 1;
                we_reject.push((name.to_string(), format!("{origin}\n      {why}")));
            }
            (false, true) => we_accept.push((name.to_string(), origin)),
        }
    }

    println!("snippets compared     : {compared}");
    println!(
        "agreed                : {agreed} / {compared} ({:.2}%)",
        percent(agreed, compared)
    );
    println!(
        "lossless              : {lossless} / {compared}{}",
        if lossless == compared { "  (all)" } else { "" }
    );
    println!(
        "sclang accepts, we do not : {}  <- bugs here",
        we_reject.len()
    );
    println!("we accept, sclang does not: {}", we_accept.len());

    if !reasons.is_empty() {
        println!("\nwhy we rejected them:");
        let mut counts: Vec<_> = reasons.into_iter().collect();
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (why, n) in counts {
            println!("  {n:4}  {why}");
        }
    }

    report("sclang accepts, we do not", &we_reject, &dir);
    report("we accept, sclang does not", &we_accept, &dir);

    // Only the first direction is a failure. The second is reported for
    // attention, not as a gate.
    std::process::exit(if we_reject.is_empty() { 0 } else { 1 });
}

fn report(label: &str, items: &[(String, String)], dir: &std::path::Path) {
    if items.is_empty() {
        return;
    }
    println!("\n---- {label} ----");
    for (name, origin) in items.iter().take(SHOW) {
        println!("  {name}  from {origin}");
        if let Ok(source) = fs::read_to_string(dir.join(name)) {
            for line in source.lines().take(3) {
                println!("      {line}");
            }
        }
    }
    if items.len() > SHOW {
        println!("  ... and {} more", items.len() - SHOW);
    }
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        100.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn reconstruct(node: &sclang_syntax::SyntaxNode, source: &str) -> String {
    let mut out = String::new();
    fn walk(node: &sclang_syntax::SyntaxNode, source: &str, out: &mut String) {
        for child in &node.children {
            match child {
                sclang_syntax::Child::Node(n) => walk(n, source, out),
                sclang_syntax::Child::Token(t) => out.push_str(t.text(source)),
            }
        }
    }
    walk(node, source, &mut out);
    out
}
