//! Idempotence and stability properties.
//!
//! A note on what is worth testing here. The naive idempotence property —
//! parse, print, reparse, compare — is *trivially* true for a lossless tree:
//! printing returns the source byte for byte, so reparsing is the same call
//! twice. One test pins it anyway, as a guard against the builder ever
//! becoming lossy, but it finds nothing on its own.
//!
//! The properties that actually find bugs are about **stability under edits**:
//!
//! - every prefix of a file parses, which is precisely what an editor asks of
//!   a parser on every keystroke;
//! - single-character mutations never panic and never lose bytes;
//! - a subtree's own text reparses to the same structure.
//!
//! These cover the case sclang's own parser cannot handle at all — incomplete
//! and invalid input — which is the reason this parser exists.

use sclang_syntax::{parse, Child, SyntaxKind, SyntaxNode};

/// Print a tree back to source.
fn print_tree(node: &SyntaxNode, source: &str, out: &mut String) {
    for child in &node.children {
        match child {
            Child::Node(n) => print_tree(n, source, out),
            Child::Token(t) => out.push_str(t.text(source)),
        }
    }
}

fn printed(src: &str) -> String {
    let p = parse(src);
    let mut out = String::new();
    print_tree(&p.root, src, &mut out);
    out
}

/// Non-trivia structure, as a flat kind sequence. Comparing this ignores
/// whitespace and comments while still pinning the shape of the tree.
fn structure(node: &SyntaxNode) -> Vec<SyntaxKind> {
    let mut out = Vec::new();
    fn walk(n: &SyntaxNode, out: &mut Vec<SyntaxKind>) {
        out.push(n.kind);
        for c in &n.children {
            match c {
                Child::Node(child) => walk(child, out),
                Child::Token(t) if !t.kind.is_trivia() => out.push(t.kind),
                Child::Token(_) => {}
            }
        }
    }
    walk(node, &mut out);
    out
}

/// A deterministic PRNG, so a failing case is always reproducible. xorshift64.
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

/// Sources exercising the constructs most likely to break under truncation.
const SAMPLES: &[&str] = &[
    "Foo : Bar { baz { ^1 } }",
    "+ Object { foo { ^this.bar(1, 2) } }",
    "Array[slot] : ArrayedCollection { at { |i| ^i } }",
    "Foo { var <a, >b, <>c; *new { |x = 1, y = (2)| ^super.new } }",
    "x = SinOsc.ar(freq: 440, mul: 0.1) * Env.perc;",
    "f = { |a b| a + b };",
    "y = (1, 3 .. 9) ++ [1, 2, \\sym, $c, 16rFF, 1e-8];",
    "~e = (freq: 440, \"dur\": 1, 0: nil);",
    "#a, b ...rest = [1, 2, 3];",
    "Foo { ++ { arg x; _Prim_Thing; ^x } }",
    "z = a[0..2] ++ b[..1] ++ c[3..];",
    "w = f.(1) + g.value { |q| q } ++ \"one \" \"two\";",
    "/* nested /* comment */ here */ v = 1; // trailing",
    "Foo { bar { while { a < b } { a = a + 1 }; ^a } }",
];

// =====================================================================
// The trivial property, pinned anyway
// =====================================================================

#[test]
fn printing_a_tree_returns_the_source() {
    for src in SAMPLES {
        assert_eq!(&printed(src), src, "print/parse round trip failed");
    }
}

#[test]
fn reparsing_printed_output_gives_an_identical_tree() {
    for src in SAMPLES {
        let once = parse(src);
        let twice = parse(&printed(src));
        assert_eq!(
            structure(&once.root),
            structure(&twice.root),
            "unstable for {src:?}"
        );
        assert_eq!(once.errors.len(), twice.errors.len());
    }
}

// =====================================================================
// Stability under truncation — the editor's real workload
// =====================================================================

/// Every prefix of every sample must parse without panicking and without
/// losing a byte. This is what the parser sees on each keystroke.
#[test]
fn every_prefix_parses_losslessly() {
    for src in SAMPLES {
        for end in 0..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let prefix = &src[..end];
            let out = printed(prefix);
            assert_eq!(
                out, prefix,
                "prefix of length {end} was not lossless: {prefix:?}"
            );
        }
    }
}

/// Truncating in the middle must not make the parser report *fewer* errors
/// than the complete text did, once the text is actually incomplete. A prefix
/// that silently parses clean would mean recovery swallowed the truncation.
#[test]
fn truncated_input_is_reported_as_erroneous() {
    // Each of these is missing a closing delimiter.
    for src in [
        "Foo { bar { ^1 ",
        "Foo : Bar {",
        "x = [1, 2",
        "y = (a: 1",
        "f = { |a ",
    ] {
        let p = parse(src);
        assert!(!p.is_ok(), "expected an error for truncated {src:?}");
        assert_eq!(&printed(src), src, "truncated input was not lossless");
    }
}

// =====================================================================
// Stability under mutation
// =====================================================================

/// Deleting any single character must leave the parser total: no panic, no
/// lost bytes.
#[test]
fn single_character_deletions_are_survivable() {
    for src in SAMPLES {
        for i in 0..src.len() {
            if !src.is_char_boundary(i) {
                continue;
            }
            let mut m = String::with_capacity(src.len());
            m.push_str(&src[..i]);
            let mut rest = src[i..].chars();
            rest.next();
            m.push_str(rest.as_str());
            assert_eq!(printed(&m), m, "deletion at {i} was not lossless: {m:?}");
        }
    }
}

/// Inserting an arbitrary delimiter anywhere must likewise be survivable.
/// Delimiters are used because they are what actually derails a parser.
#[test]
fn single_character_insertions_are_survivable() {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let chars = [
        '{', '}', '(', ')', '[', ']', ';', ',', '"', '\'', '\\', '^', '|', '*', '#',
    ];
    for src in SAMPLES {
        for _ in 0..200 {
            let mut at = rng.below(src.len() + 1);
            while !src.is_char_boundary(at) {
                at -= 1;
            }
            let c = chars[rng.below(chars.len())];
            let mut m = String::with_capacity(src.len() + 1);
            m.push_str(&src[..at]);
            m.push(c);
            m.push_str(&src[at..]);
            assert_eq!(
                printed(&m),
                m,
                "insertion of {c:?} at {at} was not lossless: {m:?}"
            );
        }
    }
}

/// Random splices of two samples: structurally nonsensical, but the parser
/// must still terminate and account for every byte.
#[test]
fn spliced_sources_are_survivable() {
    let mut rng = Rng(0x0bad_c0de_1234_5678);
    for _ in 0..400 {
        let a = SAMPLES[rng.below(SAMPLES.len())];
        let b = SAMPLES[rng.below(SAMPLES.len())];
        let mut i = rng.below(a.len() + 1);
        while !a.is_char_boundary(i) {
            i -= 1;
        }
        let mut j = rng.below(b.len() + 1);
        while !b.is_char_boundary(j) {
            j -= 1;
        }
        let m = format!("{}{}", &a[..i], &b[j..]);
        assert_eq!(printed(&m), m, "splice was not lossless: {m:?}");
    }
}

// =====================================================================
// Subtree reparse
// =====================================================================

/// A function block's own text, reparsed on its own, must produce the same
/// structure as the subtree it came from. This is the property an incremental
/// reparse would depend on, and it catches context leaking into a subtree's
/// shape.
#[test]
fn function_blocks_reparse_to_the_same_structure() {
    for src in SAMPLES {
        let p = parse(src);
        if !p.is_ok() {
            continue;
        }
        for node in p.root.descendants() {
            if node.kind != SyntaxKind::FunctionBlock {
                continue;
            }
            let text = node.text(src);
            let sub = parse(text);
            // The standalone parse wraps it in SourceFile/ExprSeq, so compare
            // the FunctionBlock found within it.
            let Some(reparsed) = sub
                .root
                .descendants()
                .into_iter()
                .find(|n| n.kind == SyntaxKind::FunctionBlock)
            else {
                panic!("subtree {text:?} did not reparse to a FunctionBlock");
            };
            assert_eq!(
                structure(node),
                structure(reparsed),
                "subtree structure changed when reparsed standalone: {text:?}"
            );
        }
    }
}
