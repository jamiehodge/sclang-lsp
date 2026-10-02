//! Diff this crate's help-file trees against upstream's own SCDoc parser.
//!
//! `oracle/build-scdoc.sh` builds upstream's standalone driver, which parses a
//! file and prints `doc_node_dump`'s rendering of the tree. [`Node::dump`]
//! prints the same format, so the comparison is of whole trees, byte for byte.
//!
//! Every file is read in two modes: the one sclang renders it with — partial
//! for an `.ext.schelp`, which adds to another class's page, and full for
//! anything else — and metadata, which is how sclang indexes every help file.
//!
//!     cargo run --release -p sclang-scdoc --example scdoc_oracle -- \
//!         oracle/scdoc_dump [--mutate N] <dir>...
//!
//! The installed help files are almost all well-formed, so on their own they
//! say little about the paths a broken file takes — and a file being edited
//! is broken most of the time. `--mutate N` also compares N damaged copies of
//! each file: a span deleted, or one of the delimiters the lexer treats
//! specially spliced in. The damage is seeded from the file's position in the
//! sorted list, so a run is reproducible.
//!
//! [`Node::dump`]: sclang_scdoc::Node::dump

use sclang_scdoc::{parse, Mode};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `--mutate` splices in: everything the lexer gives a meaning to.
const SPLICES: &[&str] = &[
    "::",
    "\n",
    "\n\n",
    "##",
    "||",
    "\\",
    ",",
    "\r",
    "\t",
    "(",
    ")",
    "code::",
    "code::\n",
    "link::",
    "method:: ",
    "argument:: ",
    "section:: ",
    "subsubsection:: ",
    "math::",
    "list::\n",
    "table::\n",
    "classmethods::\n",
    "private:: ",
    "copymethod:: ",
    "http://",
    "returns::",
    "footnote::",
];

/// How many disagreements to describe before only counting them.
const SHOWN: usize = 10;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(oracle) = args.next().map(PathBuf::from) else {
        eprintln!("usage: scdoc_oracle <scdoc_dump> [--mutate N] <dir>...");
        std::process::exit(2);
    };

    let mut mutations = 0;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--mutate" {
            mutations = args
                .next()
                .and_then(|n| n.parse().ok())
                .expect("--mutate takes a count");
        } else {
            walk(Path::new(&arg), &mut files);
        }
    }
    files.sort();
    let scratch = std::env::temp_dir().join(format!("scdoc-oracle-{}", std::process::id()));
    fs::create_dir_all(&scratch).expect("create a scratch directory");

    let (mut compared, mut agreed, mut both_rejected) = (0usize, 0usize, 0usize);
    let mut disagreements = Vec::new();

    for (n, file) in files.iter().enumerate() {
        let Ok(source) = fs::read(file) else {
            continue;
        };
        let rendered = if file.to_string_lossy().ends_with(".ext.schelp") {
            Mode::Partial
        } else {
            Mode::Full
        };

        let mut rng = Lcg(n as u64 + 1);
        let mut variants = vec![(file.clone(), source.clone())];
        for m in 0..mutations {
            let path = scratch.join(format!("{n}-{m}.schelp"));
            let damaged = mutate(&source, &mut rng);
            fs::write(&path, &damaged).expect("write a mutation");
            variants.push((path, damaged));
        }

        for (i, (path, text)) in variants.iter().enumerate() {
            for mode in [rendered, Mode::Metadata] {
                compared += 1;
                let theirs = upstream(&oracle, path, mode);
                let ours = parse(text, mode).map(|n| n.dump());
                match (&theirs, &ours) {
                    (Some(t), Ok(o)) if t == o => agreed += 1,
                    (None, Err(_)) => {
                        agreed += 1;
                        both_rejected += 1;
                    }
                    _ => {
                        // Name the original; a mutation's file is only
                        // meaningful while it still exists.
                        let label = match i {
                            0 => file.display().to_string(),
                            _ => format!("{} [mutation {i}: {}]", file.display(), path.display()),
                        };
                        disagreements.push((label, mode, theirs, ours));
                    }
                }
            }
        }
        // Keep the mutations that disagreed, for reproducing by hand.
        for (path, _) in variants.iter().skip(1) {
            if !disagreements
                .iter()
                .any(|d| d.0.contains(&*path.to_string_lossy()))
            {
                let _ = fs::remove_file(path);
            }
        }
    }

    for (file, mode, theirs, ours) in disagreements.iter().take(SHOWN) {
        println!("--- {file} ({mode:?})");
        match (theirs, ours) {
            (None, Ok(_)) => println!("    upstream rejects it, we accept it"),
            (Some(_), Err(e)) => println!("    upstream accepts it, we reject it: {e}"),
            (Some(t), Ok(o)) => {
                let line = t
                    .lines()
                    .zip(o.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| t.lines().count().min(o.lines().count()));
                println!("    trees differ from dump line {}", line + 1);
                println!("    upstream: {:?}", t.lines().nth(line).unwrap_or("<end>"));
                println!("    ours    : {:?}", o.lines().nth(line).unwrap_or("<end>"));
            }
            (None, Err(_)) => unreachable!("agreement"),
        }
    }
    if disagreements.len() > SHOWN {
        println!("... and {} more", disagreements.len() - SHOWN);
    }

    println!();
    println!("files          : {}", files.len());
    println!("mutations each : {mutations}");
    println!("parses compared: {compared}");
    println!(
        "agreed exactly : {agreed} / {compared} ({:.2}%)",
        100.0 * agreed as f64 / compared.max(1) as f64
    );
    println!("  both rejected: {both_rejected}");
    println!("disagreed      : {}", disagreements.len());

    // Only removed when empty, which is when nothing disagreed.
    let _ = fs::remove_dir(&scratch);
    if !disagreements.is_empty() {
        std::process::exit(1);
    }
}

/// Damage a file: delete a span, splice in a delimiter, or both.
fn mutate(source: &[u8], rng: &mut Lcg) -> Vec<u8> {
    let mut out = source.to_vec();
    let edits = 1 + rng.below(3);
    for _ in 0..edits {
        let at = rng.below(out.len() + 1);
        if rng.below(2) == 0 {
            let len = (1 + rng.below(16)).min(out.len() - at);
            out.drain(at..at + len);
        } else {
            let splice = SPLICES[rng.below(SPLICES.len())].as_bytes();
            out.splice(at..at, splice.iter().copied());
        }
    }
    out
}

/// Enough randomness to scatter damage, and reproducible without a
/// dependency.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Upstream's dump of `file`, or `None` if upstream rejects it.
fn upstream(oracle: &Path, file: &Path, mode: Mode) -> Option<String> {
    let mut cmd = Command::new(oracle);
    match mode {
        Mode::Full => {}
        Mode::Partial => {
            cmd.arg("--partial");
        }
        Mode::Metadata => {
            cmd.arg("--metadata");
        }
    }
    let out = cmd.arg(file).output().expect("run the SCDoc oracle");
    // Lossy for the same reason `Node` is: the comparison is of what an
    // editor would be shown, and that is UTF-8.
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "schelp") {
            out.push(path);
        }
    }
}
