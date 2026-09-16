//! Diff method resolution against sclang's own dispatch.
//!
//! `Foo(...).bar` resolves to the method an instance of `Foo` would really
//! call, found by walking `Foo`'s superclass chain. `oracle/dump-resolution.scd`
//! asks a running sclang what it would select for the same pair, using
//! `findRespondingMethodFor` — the resolution `Object` itself uses. This
//! compares the two.
//!
//!     ./oracle/run-resolution.sh
//!
//! or directly:
//!
//!     cargo run --release --example resolve_oracle -- <dump.tsv> [dir...]

use sclang_lsp::analysis::{resolve_selector, Certainty, Receiver};
use sclang_lsp::workspace::{build_index, default_roots};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How many disagreements to print before summarising the rest.
const SHOW: usize = 20;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dump) = args.next() else {
        eprintln!("usage: resolve_oracle <dump.tsv> [class library dir...]");
        std::process::exit(2);
    };

    let given: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let roots = if given.is_empty() {
        default_roots()
    } else {
        given
    };

    eprintln!("indexing {} root(s)...", roots.len());
    let (index, _references, stats) = build_index(&roots);
    eprintln!(
        "indexed {} files: {} classes, {} methods",
        stats.files, stats.classes, stats.methods
    );

    let text = std::fs::read_to_string(&dump).unwrap_or_else(|e| {
        eprintln!("could not read {dump}: {e}");
        std::process::exit(2);
    });

    let mut compared = 0usize;
    let mut agreed = 0usize;
    let mut missing_class = 0usize;
    let mut found_nothing: Vec<(String, String, String)> = Vec::new();
    let mut wrong_owner: Vec<(String, String, String, String)> = Vec::new();

    for line in text.lines() {
        let mut parts = line.split('\t');
        if parts.next() != Some("R") {
            continue;
        }
        let (Some(class), Some(selector), Some(expected)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };

        // A class the index never saw cannot be compared. The two class
        // library files that are not valid UTF-8 land here, and so would
        // anything installed since the dump was taken.
        if index.class(class).is_none() {
            missing_class += 1;
            continue;
        }

        compared += 1;
        let resolved = resolve_selector(
            &index,
            selector,
            &Receiver::Instance {
                class: class.to_string(),
                certainty: Certainty::Certain,
            },
        );

        // `single` and not `methods.first()`: the oracle asks where dispatch
        // lands, so falling back to the implementor list must count as having
        // found nothing rather than as an answer.
        match resolved.single() {
            Some(m) if m.owner == expected => agreed += 1,
            Some(m) => wrong_owner.push((
                class.to_string(),
                selector.to_string(),
                expected.to_string(),
                m.owner.clone(),
            )),
            None => found_nothing.push((
                class.to_string(),
                selector.to_string(),
                expected.to_string(),
            )),
        }
    }

    let disagreed = compared - agreed;
    println!("pairs compared   : {compared}");
    println!(
        "agreed exactly   : {agreed} / {compared} ({:.4}%)",
        percent(agreed, compared)
    );
    println!("resolved nothing : {}", found_nothing.len());
    println!("wrong owner      : {}", wrong_owner.len());
    if missing_class > 0 {
        println!("class not indexed: {missing_class} (skipped)");
    }

    report("resolved nothing", &found_nothing, |(c, s, e)| {
        format!("{c}.{s}  sclang says {e}, we found no method")
    });
    report("wrong owner", &wrong_owner, |(c, s, e, got)| {
        format!("{c}.{s}  sclang says {e}, we say {got}")
    });

    // A systematic walk error shows up as one class dominating this list,
    // where a handful of scattered names is usually an indexing gap.
    if !wrong_owner.is_empty() {
        let mut by_expected: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, _, expected, _) in &wrong_owner {
            *by_expected.entry(expected).or_default() += 1;
        }
        println!("\nwrong answers by the class sclang chose:");
        let mut counts: Vec<_> = by_expected.into_iter().collect();
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (owner, n) in counts.into_iter().take(10) {
            println!("  {owner}: {n}");
        }
    }

    std::process::exit(if disagreed == 0 { 0 } else { 1 });
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        100.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn report<T>(label: &str, items: &[T], show: impl Fn(&T) -> String) {
    if items.is_empty() {
        return;
    }
    println!("\n---- {label} ----");
    for item in items.iter().take(SHOW) {
        println!("  {}", show(item));
    }
    if items.len() > SHOW {
        println!("  ... and {} more", items.len() - SHOW);
    }
}
