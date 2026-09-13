//! Differential test of the index against sclang's own compiled class library.
//!
//! `oracle/dump-symbols.scd` asks a running sclang what classes and methods it
//! actually compiled, and where. This builds the same index from source and
//! compares. Anything sclang knows that we do not is a gap; anything we report
//! that sclang does not is a false positive.
//!
//! ```text
//! sclang -i none oracle/dump-symbols.scd
//! cargo run --release --example oracle -- oracle/oracle-symbols.tsv
//! ```

use sclang_index::{MethodKind, SymbolIndex};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A method as sclang identifies it: class, name, and side.
type Key = (String, String, char);

struct Oracle {
    classes: BTreeMap<String, String>, // name -> superclass, "-" if none
    methods: BTreeMap<Key, Vec<String>>, // key -> argument names
    files: BTreeSet<String>,
}

fn load(path: &str) -> Oracle {
    let text = std::fs::read_to_string(path).expect("could not read oracle file");
    let mut o = Oracle {
        classes: BTreeMap::new(),
        methods: BTreeMap::new(),
        files: BTreeSet::new(),
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"C") if f.len() >= 5 => {
                o.classes.insert(f[1].to_string(), f[2].to_string());
                o.files.insert(f[3].to_string());
            }
            Some(&"M") if f.len() >= 7 => {
                let kind = f[3].chars().next().unwrap_or('i');
                // sclang lists the receiver first; drop it so the comparison
                // is against declared arguments only.
                let args: Vec<String> = f[6]
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .skip(1)
                    .map(String::from)
                    .collect();
                o.methods
                    .insert((f[1].to_string(), f[2].to_string(), kind), args);
                o.files.insert(f[4].to_string());
            }
            _ => {}
        }
    }
    o
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: oracle <oracle-symbols.tsv>");
        std::process::exit(2);
    };
    let oracle = load(&path);

    // Build the real index from the same files.
    let mut index = SymbolIndex::default();
    for f in &oracle.files {
        if let Ok(src) = std::fs::read_to_string(f) {
            index.index_file(Path::new(f), &src);
        }
    }

    println!("files compared : {}", oracle.files.len());
    println!(
        "classes  oracle {:>5}   ours {:>5}",
        oracle.classes.len(),
        index.class_count()
    );
    println!(
        "methods  oracle {:>5}   ours {:>5}",
        oracle.methods.len(),
        index.method_count()
    );

    // ---- classes ----
    let missing_classes: Vec<_> = oracle
        .classes
        .keys()
        .filter(|c| index.class(c).is_none())
        .collect();
    let extra_classes: Vec<_> = index
        .classes()
        .filter(|c| !oracle.classes.contains_key(&c.name))
        .collect();
    let wrong_super: Vec<_> = oracle
        .classes
        .iter()
        .filter_map(|(name, sup)| {
            // sclang writes `-` for the one class with no superclass, and so
            // does the index. This used to skip whenever *we* had none, which
            // made the check blind to the very thing it should have caught:
            // for years the index left an unwritten superclass implicit, and
            // every chain through such a class stopped short of Object.
            let mine = index.class(name)?.superclass.as_deref().unwrap_or("-");
            (mine != sup).then(|| (name.clone(), sup.clone(), mine.to_string()))
        })
        .collect();

    println!("\n-- classes --");
    println!("  missing : {}", missing_classes.len());
    for c in missing_classes.iter().take(10) {
        println!("      {c}");
    }
    println!("  extra   : {}", extra_classes.len());
    for c in extra_classes.iter().take(10) {
        println!("      {}", c.name);
    }
    println!("  superclass mismatches: {}", wrong_super.len());
    for (n, o, m) in wrong_super.iter().take(10) {
        println!("      {n}: sclang={o} ours={m}");
    }

    // ---- methods ----
    let ours_keys: BTreeSet<Key> = index
        .methods()
        .map(|m| (m.owner.clone(), m.name.clone(), m.kind.as_char()))
        .collect();
    let missing: Vec<_> = oracle
        .methods
        .keys()
        .filter(|k| !ours_keys.contains(*k))
        .collect();
    let extra: Vec<_> = ours_keys
        .iter()
        .filter(|k| !oracle.methods.contains_key(*k))
        .collect();

    println!("\n-- methods --");
    println!("  missing : {}", missing.len());
    for (c, m, k) in missing.iter().take(15) {
        println!("      {c}.{m} ({k})");
    }
    println!("  extra   : {}", extra.len());
    for (c, m, k) in extra.iter().take(15) {
        println!("      {c}.{m} ({k})");
    }

    // ---- argument names ----
    let mut arg_mismatch = Vec::new();
    for ((owner, name, kind), theirs) in &oracle.methods {
        let k = if *kind == 'c' {
            MethodKind::Class
        } else {
            MethodKind::Instance
        };
        let Some(m) = index.method(owner, name, k) else {
            continue;
        };
        let mine: Vec<String> = m.args.iter().map(|a| a.name.clone()).collect();
        if &mine != theirs {
            arg_mismatch.push(((owner.clone(), name.clone()), theirs.clone(), mine));
        }
    }
    println!("\n-- argument names --");
    println!(
        "  mismatches: {} of {}",
        arg_mismatch.len(),
        oracle.methods.len()
    );
    for ((c, m), o, mine) in arg_mismatch.iter().take(15) {
        println!("      {c}.{m}\n        sclang: {o:?}\n        ours  : {mine:?}");
    }

    let agree = oracle.methods.len() - missing.len() - arg_mismatch.len();
    println!(
        "\nmethods matching sclang exactly (name and arguments): {agree} / {} ({:.2}%)",
        oracle.methods.len(),
        100.0 * agree as f64 / oracle.methods.len().max(1) as f64
    );
}
