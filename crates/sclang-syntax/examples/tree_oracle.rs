//! Differential test of expression *structure* against sclang's own parse tree.
//!
//! `oracle/expose-dumpparsenode.patch` makes sclang emit the parse tree for
//! every class-library file. This compares that against ours.
//!
//! Node-for-node comparison is not possible: sclang's IR is desugared for a
//! code generator — `Drop` nodes sequence statements, `PushLit`/`PushName`
//! load operands, and `mNext` chains flatten siblings onto one level. What
//! *is* comparable is the **pre-order sequence of selectors** in each method
//! body: which messages are sent, in which order, at which nesting.
//!
//! That sequence is sensitive to the thing that matters most. `(1 + 2) * 3`
//! emits `*` then `+`; `1 + (2 * 3)` emits `+` then `*`. So it detects a
//! precedence error, which is exactly the class of bug that parses cleanly and
//! stays wrong.
//!
//! ```text
//! SCLANG_DUMP_PARSE=1 <sclang> -a -l conf.yaml -i none quit.scd > dump.log
//! cargo run --release --example tree_oracle -- dump.log
//! ```

use sclang_syntax::{parse, Child, SyntaxKind, SyntaxNode};
use std::collections::BTreeMap;

/// Selectors for one method, keyed by `Class.method`.
type Methods = BTreeMap<String, Vec<String>>;

// =====================================================================
// sclang's side: read the dump
// =====================================================================

/// Parse `dump.log` into per-file, per-method selector sequences.
fn read_dump(text: &str) -> BTreeMap<String, Methods> {
    let mut files: BTreeMap<String, Methods> = BTreeMap::new();
    let mut file: Option<String> = None;
    let mut class = String::new();
    let mut method: Option<(String, i32, bool)> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("***PARSE-DUMP-BEGIN ") {
            file = rest.strip_suffix("***").map(|s| s.to_string());
            class.clear();
            method = None;
            continue;
        }
        if line.starts_with("***PARSE-DUMP-END") {
            file = None;
            continue;
        }
        let Some(f) = &file else { continue };

        // Dump lines are `<level> <Kind>[ 'name']`, indented by a %2d level.
        let trimmed = line.trim_start();
        let Some((level_str, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        let Ok(level) = level_str.parse::<i32>() else {
            continue;
        };
        let rest = rest.trim_start();
        let (kind, name) = match rest.split_once(' ') {
            Some((k, n)) => (k, n.trim().trim_matches('\'').to_string()),
            None => (rest, String::new()),
        };

        match kind {
            "Class" | "ClassExt" => {
                class = name;
                method = None;
            }
            "MethodNode" => {
                // The name is followed by an optional primitive name.
                let name = name.split('\'').next().unwrap_or(&name).trim().to_string();
                let key = format!("{class}.{name}");
                // compileClass runs once per class, but dumpNodeList follows
                // mNext — so each call re-dumps the rest of the chain and a
                // file's later classes appear in several sections. Keep the
                // first sighting of each method and ignore the repeats.
                let already = files.get(f).is_some_and(|m| m.contains_key(&key));
                method = Some((name, level, already));
                if !already {
                    files.entry(f.clone()).or_default().entry(key).or_default();
                }
            }
            "Call" | "BinopCall" => {
                if let Some((m, mlevel, skip)) = &method {
                    if !skip && level > *mlevel {
                        let key = format!("{class}.{m}");
                        files
                            .entry(f.clone())
                            .or_default()
                            .entry(key)
                            .or_default()
                            .push(name);
                    }
                }
            }
            _ => {}
        }
    }
    files
}

// =====================================================================
// Our side: the same sequence from the CST
// =====================================================================

/// Emit selectors in sclang's order, applying the rewrites its parser performs.
///
/// sclang dumps the selector *before* its arguments, and the receiver is the
/// first argument — so `a.foo.bar` is `bar` then `foo`. Our tree nests the
/// other way round, so the selector is emitted before descending.
///
/// It also rewrites several forms while parsing, each verified against the
/// dump directly:
///
/// | source            | sclang emits        |
/// |-------------------|---------------------|
/// | `arr[3]`          | `Call 'at'`         |
/// | `arr[3] = 9`      | `Call 'put'`        |
/// | `Point(1, 2)`     | `Call 'new'`        |
/// | `this.foo(*args)` | `Call 'performList'`|
/// | `obj.bar = 7`     | nothing             |
/// | `arr[1..3]`       | `Call 'copySeries'` |
/// | `(1..10)`         | `Call 'prSimpleNumberSeries'` |
/// | `super.f(*args)`  | `Call 'superPerformList'` |
/// | `f.(1)`           | `Call 'value'`      |
/// | `~x`              | `Call 'envirGet'`   |
/// | `~x = 5`          | `Call 'envirPut'`   |
fn selectors(node: &SyntaxNode, source: &str, out: &mut Vec<String>) {
    match node.kind {
        SyntaxKind::MethodCall => {
            // `f.(1)` has no selector after the dot; sclang calls it `value`.
            if after_dot(node, source).is_none()
                && node.child_tokens().any(|t| t.kind == SyntaxKind::Dot)
            {
                out.push("value".to_string());
                descend(node, source, out);
                return;
            }
            if let Some(name) = after_dot(node, source) {
                // `f(*args)` becomes `performList`, the selector moving into
                // an argument — `superPerformList` when the receiver is
                // `super`.
                out.push(if has_splat(node) {
                    if receiver_is_super(node, source) {
                        "superPerformList".to_string()
                    } else {
                        "performList".to_string()
                    }
                } else {
                    name
                });
            }
            descend(node, source, out);
            return;
        }
        SyntaxKind::BinaryExpr => {
            if let Some(op) = operator_of(node, source) {
                out.push(op);
            }
            descend(node, source, out);
            return;
        }
        SyntaxKind::CallExpr => {
            if let Some(Child::Node(first)) = node.children.first() {
                match first.kind {
                    // `Point(1, 2)` is an implicit `new` on the class, and
                    // `Point(*args)` expands like any other call.
                    SyntaxKind::ClassRef => out.push(
                        if has_splat(node) {
                            "performList"
                        } else {
                            "new"
                        }
                        .to_string(),
                    ),
                    SyntaxKind::NameRef => {
                        if let Some(t) = first.child_tokens().find(|t| !t.kind.is_trivia()) {
                            let name = t.text(source).to_string();
                            out.push(if has_splat(node) {
                                "performList".to_string()
                            } else {
                                name
                            });
                        }
                    }
                    _ => {}
                }
            }
            descend(node, source, out);
            return;
        }
        // `~x` is an environment lookup.
        SyntaxKind::EnvVarRef => {
            out.push("envirGet".to_string());
            return;
        }
        // `arr[3]` is `at`; a subrange `arr[1..3]` is `copySeries`. But
        // `Set[1, 2]` is a typed collection *literal*, not an index, and
        // builds a DynList with no call of its own.
        SyntaxKind::IndexExpr | SyntaxKind::DotIndexExpr => {
            let typed_literal = matches!(node.children.first(), Some(Child::Node(n))
                if n.kind == SyntaxKind::ClassRef);
            if !typed_literal {
                let subrange = node.child_nodes().any(|n| n.kind == SyntaxKind::IndexRange);
                out.push(if subrange { "copySeries" } else { "at" }.to_string());
            }
            descend(node, source, out);
            return;
        }
        // `(1..10)` and `(1, 3..9)` are a primitive series constructor, and
        // its bounds are arguments rather than nested calls.
        SyntaxKind::ArithSeries => {
            out.push("prSimpleNumberSeries".to_string());
            descend(node, source, out);
            return;
        }
        SyntaxKind::AssignExpr => {
            // The rewrite depends on what is being assigned to.
            if let Some(Child::Node(target)) = node.children.first() {
                match target.kind {
                    // `arr[3] = 9` is `put`, replacing the `at` the index
                    // would otherwise contribute; a subrange is `putSeries`.
                    SyntaxKind::IndexExpr | SyntaxKind::DotIndexExpr => {
                        let subrange = target
                            .child_nodes()
                            .any(|n| n.kind == SyntaxKind::IndexRange);
                        out.push(if subrange { "putSeries" } else { "put" }.to_string());
                        for c in &target.children {
                            if let Child::Node(n) = c {
                                selectors(n, source, out);
                            }
                        }
                        for c in node.children.iter().skip(1) {
                            if let Child::Node(n) = c {
                                selectors(n, source, out);
                            }
                        }
                        return;
                    }
                    // `~x = 5` is an environment store, not a read.
                    SyntaxKind::EnvVarRef => {
                        out.push("envirPut".to_string());
                        for c in node.children.iter().skip(1) {
                            if let Child::Node(n) = c {
                                selectors(n, source, out);
                            }
                        }
                        return;
                    }
                    // `obj.bar = 7` emits no call of its own.
                    SyntaxKind::MethodCall => {
                        for c in &target.children {
                            if let Child::Node(n) = c {
                                selectors(n, source, out);
                            }
                        }
                        for c in node.children.iter().skip(1) {
                            if let Child::Node(n) = c {
                                selectors(n, source, out);
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
            descend(node, source, out);
            return;
        }
        _ => {}
    }
    descend(node, source, out);
}

fn descend(node: &SyntaxNode, source: &str, out: &mut Vec<String>) {
    for c in &node.children {
        if let Child::Node(n) = c {
            selectors(n, source, out);
        }
    }
}

/// Whether the receiver is the literal `super`.
fn receiver_is_super(node: &SyntaxNode, source: &str) -> bool {
    matches!(node.children.first(), Some(Child::Node(n))
        if n.kind == SyntaxKind::NameRef
            && n.child_tokens().any(|t| t.text(source) == "super"))
}

/// Whether a call passes an expanded array, which sclang turns into
/// `performList`.
fn has_splat(node: &SyntaxNode) -> bool {
    node.child_nodes()
        .filter(|n| n.kind == SyntaxKind::ArgList)
        .any(|args| args.child_nodes().any(|a| a.kind == SyntaxKind::SplatArg))
}

fn after_dot(node: &SyntaxNode, source: &str) -> Option<String> {
    let mut seen_dot = false;
    for c in &node.children {
        if let Child::Token(t) = c {
            if t.kind.is_trivia() {
                continue;
            }
            if t.kind == SyntaxKind::Dot {
                seen_dot = true;
            } else if seen_dot {
                return Some(t.text(source).to_string());
            }
        }
    }
    None
}

fn operator_of(node: &SyntaxNode, source: &str) -> Option<String> {
    node.child_tokens()
        .find(|t| t.kind.is_operator())
        .map(|t| t.text(source).trim_end_matches(':').to_string())
}

/// Our per-method selector sequences for one file.
fn ours(source: &str) -> Methods {
    let parsed = parse(source);
    let mut out = Methods::new();
    for node in parsed.root.descendants() {
        let class = match node.kind {
            SyntaxKind::ClassDef => name_of(node, source, false),
            SyntaxKind::ClassExtension => name_of(node, source, true),
            _ => continue,
        };
        let Some(class) = class else { continue };
        for m in node.descendants() {
            if m.kind != SyntaxKind::MethodDef {
                continue;
            }
            let toks: Vec<_> = m.child_tokens().filter(|t| !t.kind.is_trivia()).collect();
            let is_class = matches!(toks.first(), Some(t) if t.kind == SyntaxKind::Star)
                && matches!(toks.get(1), Some(t) if t.kind != SyntaxKind::LBrace);
            let Some(name) = name_of(m, source, is_class) else {
                continue;
            };
            // The dump marks class methods with `*`; a class and instance
            // method of the same name are otherwise indistinguishable.
            let name = if is_class { format!("*{name}") } else { name };
            let mut sels = Vec::new();
            // sclang dumps mArglist, then mVarlist, then mBody — and argument
            // defaults can contain calls, so the argument list belongs in the
            // comparison rather than being skipped.
            for c in &m.children {
                if let Child::Node(n) = c {
                    selectors(n, source, &mut sels);
                }
            }
            out.insert(format!("{class}.{name}"), sels);
        }
    }
    out
}

fn name_of(node: &SyntaxNode, source: &str, skip_lead: bool) -> Option<String> {
    let mut skipped = false;
    for c in &node.children {
        if let Child::Token(t) = c {
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

// =====================================================================

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tree_oracle <dump.log>");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).expect("could not read dump");
    let dump = read_dump(&text);

    let (mut files, mut methods, mut agreed) = (0usize, 0usize, 0usize);
    let mut selector_total = 0usize;
    let mut only_theirs = 0usize;
    let mut diffs: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples: Vec<String> = Vec::new();

    for (file, theirs) in &dump {
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        files += 1;
        let mine = ours(&src);

        for (key, their_sels) in theirs {
            methods += 1;
            selector_total += their_sels.len();
            let Some(my_sels) = mine.get(key) else {
                only_theirs += 1;
                continue;
            };
            if my_sels == their_sels {
                agreed += 1;
                continue;
            }
            // Classify by the first position where they diverge.
            let at = their_sels
                .iter()
                .zip(my_sels.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(their_sels.len().min(my_sels.len()));
            let t = their_sels.get(at).map(String::as_str).unwrap_or("<end>");
            let m = my_sels.get(at).map(String::as_str).unwrap_or("<end>");
            *diffs.entry(format!("sclang={t} ours={m}")).or_default() += 1;
            let want = std::env::args().nth(2);
            let show = want.as_deref().map_or(true, |w| {
                key.contains(w) || format!("sclang={t} ours={m}").contains(w)
            });
            if show && samples.len() < 15 {
                samples.push(format!(
                    "{key}\n      sclang: {:?}\n      ours  : {:?}",
                    &their_sels[..their_sels.len().min(at + 4)],
                    &my_sels[..my_sels.len().min(at + 4)]
                ));
            }
        }
    }

    println!("files            : {files}");
    println!("methods compared : {methods}");
    println!("selectors compared: {selector_total}");
    println!("not found in ours: {only_theirs}");
    println!(
        "selector sequences identical: {agreed} / {methods} ({:.2}%)",
        100.0 * agreed as f64 / methods.max(1) as f64
    );

    if !diffs.is_empty() {
        let mut d: Vec<_> = diffs.into_iter().collect();
        d.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\nfirst divergence, by kind:");
        for (k, n) in d.iter().take(15) {
            println!("  {n:>6}  {k}");
        }
        println!("\nsamples:");
        for s in samples.iter().take(10) {
            println!("  {s}");
        }
    }
}
