//! Differential test against sclang's own compiled class library.
//!
//! `oracle/dump-symbols.scd` asks a running sclang what classes and methods it
//! actually compiled, and where. This tool parses the same files and compares.
//! Anything sclang knows that we do not is a gap in the parser; anything we
//! report that sclang does not is a false positive.
//!
//! ```text
//! sclang -i none oracle/dump-symbols.scd          # writes oracle-symbols.tsv
//! cargo run --release --example oracle -- oracle/oracle-symbols.tsv
//! ```

use sclang_syntax::{parse, Child, SyntaxKind, SyntaxNode};
use std::collections::{BTreeMap, BTreeSet};

/// A method identified the way sclang identifies it: class, name, and whether
/// it is a class method.
type MethodKey = (String, String, char);

#[derive(Default)]
struct Symbols {
    classes: BTreeMap<String, String>, // name -> superclass ("-" if none)
    methods: BTreeMap<MethodKey, Vec<String>>, // key -> argument names
}

/// Read the TSV written by `dump-symbols.scd`, keeping only the files we were
/// asked to check.
fn load_oracle(path: &str) -> (Symbols, BTreeSet<String>) {
    let text = std::fs::read_to_string(path).expect("could not read oracle file");
    let mut syms = Symbols::default();
    let mut files = BTreeSet::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"C") if f.len() >= 5 => {
                syms.classes.insert(f[1].to_string(), f[2].to_string());
                files.insert(f[3].to_string());
            }
            Some(&"M") if f.len() >= 7 => {
                let kind = f[3].chars().next().unwrap_or('i');
                // sclang lists the receiver as the first argument; drop it so
                // the comparison is against declared arguments only.
                let args: Vec<String> = f[6]
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .skip(1)
                    .map(|s| s.to_string())
                    .collect();
                syms.methods
                    .insert((f[1].to_string(), f[2].to_string(), kind), args);
                files.insert(f[4].to_string());
            }
            _ => {}
        }
    }
    (syms, files)
}

/// The identifying token of a node: the class name, or the method name.
///
/// `skip_lead` drops one leading marker token — the `*` of a class method, or
/// the `+` of a class extension.
fn name_token(node: &SyntaxNode, source: &str, skip_lead: bool) -> Option<String> {
    let mut skipped = false;
    for child in &node.children {
        if let Child::Token(t) = child {
            if t.kind.is_trivia() {
                continue;
            }
            if skip_lead && !skipped && matches!(t.kind, SyntaxKind::Star | SyntaxKind::Plus) {
                skipped = true;
                continue;
            }
            return Some(t.text(source).to_string());
        }
    }
    None
}

/// sclang synthesises accessor methods from the `<`, `>` and `<>` markers on a
/// variable declaration: `var <>foo` yields `foo` and `foo_`, `var <foo` only
/// the getter. These are real methods — completion has to offer them — so we
/// synthesise them too rather than excluding them from the comparison.
///
/// `classvar` accessors live on the class side, `var` accessors on instances.
fn add_synthesised_accessors(
    class_node: &SyntaxNode,
    class_name: &str,
    source: &str,
    syms: &mut Symbols,
) {
    for decl in class_node.descendants() {
        if decl.kind != SyntaxKind::ClassVarDecl {
            continue;
        }
        // `classvar` and `const` accessors live on the class side; `var`
        // accessors on instances. `const <comma = $,` in Char.sc is a class
        // method, not an instance one.
        let class_side = decl
            .child_tokens()
            .any(|t| matches!(t.kind, SyntaxKind::ClassvarKw | SyntaxKind::ConstKw));
        let kind = if class_side { 'c' } else { 'i' };

        for slot in decl.child_nodes().filter(|n| n.kind == SyntaxKind::SlotDef) {
            let Some(spec) = slot.child_of(SyntaxKind::RwSpec) else {
                continue;
            };
            let Some(name) = slot
                .child_tokens()
                .find(|t| t.kind == SyntaxKind::Ident)
                .map(|t| t.text(source).to_string())
            else {
                continue;
            };
            let marker = spec
                .child_tokens()
                .find(|t| !t.kind.is_trivia())
                .map(|t| t.kind);
            let (getter, setter) = match marker {
                Some(SyntaxKind::Lt) => (true, false),
                Some(SyntaxKind::Gt) => (false, true),
                Some(SyntaxKind::ReadWriteVar) => (true, true),
                _ => (false, false),
            };
            if getter {
                syms.methods
                    .entry((class_name.to_string(), name.clone(), kind))
                    .or_default();
            }
            if setter {
                syms.methods
                    .entry((class_name.to_string(), format!("{name}_"), kind))
                    .or_default();
            }
        }
    }
}

/// Whether a `MethodDef` is a class method: a leading `*` followed by a *name*.
///
/// `* { arg a; ... }` is an instance method named `*` (AbstractFunction.sc:98),
/// so a `*` immediately followed by the body brace is not the class marker.
fn is_class_method(node: &SyntaxNode) -> bool {
    let toks: Vec<_> = node
        .child_tokens()
        .filter(|t| !t.kind.is_trivia())
        .collect();
    matches!(toks.first(), Some(t) if t.kind == SyntaxKind::Star)
        && matches!(toks.get(1), Some(t) if t.kind != SyntaxKind::LBrace)
}

/// Argument names declared by a method, in order.
fn arg_names(method: &SyntaxNode, source: &str) -> Vec<String> {
    // Direct child only: `descendants()` would reach into nested function
    // blocks and borrow their arguments for a method that declares none.
    let Some(decls) = method.child_of(SyntaxKind::ArgDecls) else {
        return Vec::new();
    };
    decls
        .child_nodes()
        .filter(|n| matches!(n.kind, SyntaxKind::VarDef | SyntaxKind::RestArg))
        .filter_map(|n| {
            n.child_tokens()
                .find(|t| t.kind == SyntaxKind::Ident)
                .map(|t| t.text(source).to_string())
        })
        .collect()
}

/// Walk a parsed file and collect the symbols it declares.
fn collect_from_file(path: &str, syms: &mut Symbols) {
    let Ok(src) = std::fs::read_to_string(path) else {
        return;
    };
    let parsed = parse(&src);

    for node in parsed.root.descendants() {
        let (class_name, is_extension) = match node.kind {
            SyntaxKind::ClassDef => (name_token(node, &src, false), false),
            // `+ Foo { ... }` adds methods to an existing class, which is how
            // sclang reports them too.
            SyntaxKind::ClassExtension => (name_token(node, &src, true), true),
            _ => continue,
        };
        let Some(class_name) = class_name else {
            continue;
        };

        if !is_extension {
            add_synthesised_accessors(node, &class_name, &src, syms);
            let superclass = node
                .child_of(SyntaxKind::SuperClass)
                .and_then(|s| {
                    s.child_tokens()
                        .find(|t| t.kind == SyntaxKind::ClassName)
                        .map(|t| t.text(&src).to_string())
                })
                .unwrap_or_else(|| "-".to_string());
            syms.classes.insert(class_name.clone(), superclass);
        }

        for m in node.descendants() {
            if m.kind != SyntaxKind::MethodDef {
                continue;
            }
            let class_method = is_class_method(m);
            let Some(name) = name_token(m, &src, class_method) else {
                continue;
            };
            let kind = if class_method { 'c' } else { 'i' };
            syms.methods
                .insert((class_name.clone(), name, kind), arg_names(m, &src));
        }
    }
}

fn main() {
    let Some(oracle_path) = std::env::args().nth(1) else {
        eprintln!("usage: oracle <oracle-symbols.tsv>");
        std::process::exit(2);
    };
    let (oracle, files) = load_oracle(&oracle_path);

    let mut ours = Symbols::default();
    for f in &files {
        collect_from_file(f, &mut ours);
    }

    println!("files compared     : {}", files.len());
    println!(
        "classes  oracle {:>5}   ours {:>5}",
        oracle.classes.len(),
        ours.classes.len()
    );
    println!(
        "methods  oracle {:>5}   ours {:>5}",
        oracle.methods.len(),
        ours.methods.len()
    );

    // ---- classes ----
    let missing_classes: Vec<_> = oracle
        .classes
        .keys()
        .filter(|c| !ours.classes.contains_key(*c))
        .collect();
    let extra_classes: Vec<_> = ours
        .classes
        .keys()
        .filter(|c| !oracle.classes.contains_key(*c))
        .collect();
    let wrong_super: Vec<_> = oracle
        .classes
        .iter()
        .filter_map(|(name, sup)| {
            let mine = ours.classes.get(name)?;
            // sclang resolves the implicit superclass (Object) that the source
            // leaves unwritten, so only compare when we recorded one.
            (mine != "-" && mine != sup).then(|| (name.clone(), sup.clone(), mine.clone()))
        })
        .collect();

    println!("\n-- classes --");
    println!(
        "  missing (sclang has, we don't) : {}",
        missing_classes.len()
    );
    for c in missing_classes.iter().take(10) {
        println!("      {c}");
    }
    println!(
        "  extra   (we have, sclang doesn't): {}",
        extra_classes.len()
    );
    for c in extra_classes.iter().take(10) {
        println!("      {c}");
    }
    println!("  superclass mismatches           : {}", wrong_super.len());
    for (n, o, m) in wrong_super.iter().take(10) {
        println!("      {n}: sclang={o} ours={m}");
    }

    // ---- methods ----
    let missing: Vec<_> = oracle
        .methods
        .keys()
        .filter(|k| !ours.methods.contains_key(*k))
        .collect();
    let extra: Vec<_> = ours
        .methods
        .keys()
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
    for (key, theirs) in &oracle.methods {
        if let Some(mine) = ours.methods.get(key) {
            if mine != theirs {
                arg_mismatch.push((key.clone(), theirs.clone(), mine.clone()));
            }
        }
    }
    println!("\n-- argument names --");
    println!(
        "  mismatches: {} of {}",
        arg_mismatch.len(),
        oracle.methods.len()
    );
    for ((c, m, _), o, mine) in arg_mismatch.iter().take(15) {
        println!("      {c}.{m}\n        sclang: {o:?}\n        ours  : {mine:?}");
    }

    let agree = oracle.methods.len() - missing.len() - arg_mismatch.len();
    println!(
        "\nmethods matching sclang exactly (name and arguments): {agree} / {} ({:.2}%)",
        oracle.methods.len(),
        100.0 * agree as f64 / oracle.methods.len().max(1) as f64
    );
}
