//! Properties of the indenter.
//!
//! The contract is narrow enough to state exactly: **only the leading
//! whitespace of a line ever changes.** Everything here is a way of checking
//! that, or of checking that repeating the operation does nothing.
//!
//! Unlike the printer's round trip — trivially true for a lossless tree, as
//! `idempotence.rs` says at its head — neither property here is free. The
//! indenter computes new text, so both can genuinely break.

use sclang_syntax::{parse_with, reindent, Child, IndentStyle, Mode, SyntaxKind, SyntaxNode};

fn indented(source: &str, mode: Mode) -> String {
    reindent(source, &parse_with(source, mode), IndentStyle::default())
}

/// Every line with its leading whitespace removed.
///
/// Two sources agreeing on this differ in indentation and in nothing else,
/// which is the whole contract in one comparison.
fn stripped(source: &str) -> Vec<&str> {
    source.lines().map(str::trim_start).collect()
}

/// Non-trivia structure, as a flat kind sequence — the same helper
/// `idempotence.rs` uses, for the same reason: it pins the shape of the tree
/// while ignoring the whitespace we are deliberately changing.
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

/// Multi-line sources, since indentation is only interesting across lines.
/// Deliberately a mix of well-indented, flat and hostile.
const SAMPLES: &[(&str, Mode)] = &[
    (
        "Foo : Bar {\nvar <a, >b;\nbaz { |x = 1|\n^x + 1\n}\n}\n",
        Mode::ClassFile,
    ),
    (
        "+ Object {\n    foo {\n        ^this.bar(1, 2)\n    }\n}\n",
        Mode::ClassFile,
    ),
    (
        "(\nvar x = 1;\nx.postln;\n)\n",
        Mode::Script,
    ),
    (
        "(\nPbind(\n\\degree, Pseq([0, 2, 4], inf),\n\\dur,    0.25\n).play;\n)\n",
        Mode::Script,
    ),
    (
        "(\nSinOsc.ar(440)\n    + Saw.ar(220)\n    * 0.1;\n)\n",
        Mode::Script,
    ),
    (
        "(\nf = { |a, b|\n    a | b\n};\ng = #{\n1\n};\n)\n",
        Mode::Script,
    ),
    (
        "(\n$(;\n\")\";\n'(';\n// (\n/* /* */ */\n1;\n)\n",
        Mode::Script,
    ),
    (
        "(\nx = \"line one\nline two\";\ny = [1, 2,\n3];\n)\n",
        Mode::Script,
    ),
    (
        "(\n#a, b ...rest = [1, 2, 3];\nz = a[0..2] ++ b[..1];\n~e = (freq: 440,\n\"dur\": 1);\n)\n",
        Mode::Script,
    ),
    (
        "Foo {\nbar {\nwhile { a < b } {\na = a + 1\n};\n^a\n}\n}\n",
        Mode::ClassFile,
    ),
];

// =====================================================================
// The contract
// =====================================================================

/// Indenting changes leading whitespace and nothing else.
#[test]
fn only_leading_whitespace_changes() {
    for (src, mode) in SAMPLES {
        let out = indented(src, *mode);
        assert_eq!(
            stripped(&out),
            stripped(src),
            "content changed for {src:?} -> {out:?}"
        );
    }
}

/// And it changes no line's existence: an indenter never adds or removes one.
#[test]
fn the_line_count_is_unchanged() {
    for (src, mode) in SAMPLES {
        let out = indented(src, *mode);
        assert_eq!(
            out.bytes().filter(|b| *b == b'\n').count(),
            src.bytes().filter(|b| *b == b'\n').count(),
            "line count changed for {src:?} -> {out:?}"
        );
    }
}

/// No token moved, changed or vanished.
#[test]
fn the_tree_keeps_its_shape() {
    for (src, mode) in SAMPLES {
        let out = indented(src, *mode);
        assert_eq!(
            structure(&parse_with(&out, *mode).root),
            structure(&parse_with(src, *mode).root),
            "structure changed for {src:?} -> {out:?}"
        );
    }
}

// =====================================================================
// Idempotence — not free here, unlike the printer's round trip
// =====================================================================

/// Indenting an already-indented file does nothing. Format-on-save runs this
/// over and over on the same text, so a file that oscillates would be obvious
/// and maddening.
#[test]
fn indenting_twice_is_indenting_once() {
    for (src, mode) in SAMPLES {
        let once = indented(src, *mode);
        let twice = indented(&once, *mode);
        assert_eq!(twice, once, "not a fixed point for {src:?}");
    }
}

// =====================================================================
// Stability under edits — the editor's real workload
// =====================================================================

/// Every prefix of every sample must indent without panicking and without
/// changing anything but leading whitespace. This is the state a buffer is in
/// on most keystrokes.
#[test]
fn every_prefix_is_survivable() {
    for (src, mode) in SAMPLES {
        for end in 0..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let prefix = &src[..end];
            let out = indented(prefix, *mode);
            assert_eq!(
                stripped(&out),
                stripped(prefix),
                "prefix of len {end} changed content: {prefix:?} -> {out:?}"
            );
        }
    }
}

/// Single-character insertions, from the alphabet that actually derails a
/// parser. A damaged tree must still produce a total answer.
#[test]
fn single_character_insertions_are_survivable() {
    let mut rng = Rng(0x1dea_beef_1234_5678);
    let chars = [
        '{', '}', '(', ')', '[', ']', ';', ',', '"', '\'', '\\', '^', '|', '*', '#', '\n', '\t',
    ];
    for (src, mode) in SAMPLES {
        for _ in 0..200 {
            let mut at = rng.below(src.len() + 1);
            while !src.is_char_boundary(at) {
                at -= 1;
            }
            let c = chars[rng.below(chars.len())];
            let mutated = format!("{}{}{}", &src[..at], c, &src[at..]);
            let out = indented(&mutated, *mode);
            assert_eq!(
                stripped(&out),
                stripped(&mutated),
                "insertion of {c:?} at {at} changed content: {mutated:?} -> {out:?}"
            );
        }
    }
}

/// Random splices of two samples: structurally nonsensical, but the indenter
/// must still terminate and leave the content alone.
#[test]
fn spliced_sources_are_survivable() {
    let mut rng = Rng(0x0bad_c0de_9876_5432);
    for _ in 0..400 {
        let (a, mode) = SAMPLES[rng.below(SAMPLES.len())];
        let (b, _) = SAMPLES[rng.below(SAMPLES.len())];
        let mut i = rng.below(a.len() + 1);
        while !a.is_char_boundary(i) {
            i -= 1;
        }
        let mut j = rng.below(b.len() + 1);
        while !b.is_char_boundary(j) {
            j -= 1;
        }
        let spliced = format!("{}{}", &a[..i], &b[j..]);
        let out = indented(&spliced, mode);
        assert_eq!(
            stripped(&out),
            stripped(&spliced),
            "splice changed content: {spliced:?} -> {out:?}"
        );
    }
}

// =====================================================================
// Degenerate input
// =====================================================================

#[test]
fn nothing_at_all_is_fine() {
    for source in ["", "\n", "   ", "\n\n\n", "\r\n"] {
        let out = indented(source, Mode::Script);
        assert_eq!(
            stripped(&out),
            stripped(source),
            "content changed for {source:?} -> {out:?}"
        );
    }
}
