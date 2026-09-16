//! Every feature, over broken input, without falling over.
//!
//! A language server spends most of its life looking at code that is not
//! finished, and a panic in one request kills the process rather than the
//! request: `Server::run` owns the event loop on the calling thread, so the
//! editor loses its server and every open buffer loses its diagnostics.
//!
//! The per-feature tests all use code that parses. This one covers the other
//! case. It is a sweep rather than a list of cases because the failures it
//! found were not ones anybody would have thought to write down — one of them
//! was `Foo { var a = #; }`, where a node in the tree ended before it started
//! and reading its text panicked the indexer on the next keystroke.
//!
//! Two things are asserted, and neither is about the answers: every request
//! must return, and every range it returns must be one the editor can apply.
//! What the answers *say* is the other tests' business.

use lsp_types::{FormattingOptions, Position, Range, Url};
use sclang_index::SymbolIndex;
use sclang_lsp::documents::Document;
use sclang_lsp::features;
use sclang_lsp::line_index::PositionEncoding;
use sclang_lsp::references::ReferenceIndex;
use std::path::Path;

const ENC: PositionEncoding = PositionEncoding::Utf16;

/// Sources exercising the constructs most likely to break under mutation.
/// The same corpus the syntax crate's stability tests use, plus the class-var
/// initialiser that started this.
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
    "Foo { var a = #; }",
    "Foo { classvar all; *initClass { all = IdentityDictionary.new } }",
];

/// Ask every request this server answers, at every position in the file.
///
/// The document is also indexed, because indexing is what `didChange` does
/// before anything else and it is on the same thread.
fn exercise(source: &str) {
    let doc = Document::new(source.to_string(), 1);
    let path = Path::new("a.sc");
    let uri = Url::parse("file:///a.sc").expect("a valid test uri");

    let mut index = SymbolIndex::default();
    index.index_file(path, source);
    let mut references = ReferenceIndex::default();
    references.index_file(path, source);

    // The whole document, in the shape a client asks for a viewport.
    let whole = Range::new(Position::new(0, 0), Position::new(u32::MAX, 0));
    let options = FormattingOptions {
        tab_size: 4,
        insert_spaces: true,
        ..Default::default()
    };

    // Document-wide requests.
    for range in features::diagnostics::diagnostics(&doc, ENC)
        .iter()
        .map(|d| d.range)
        .chain(
            features::folding_range::folding_ranges(&doc)
                .iter()
                .map(|f| Range::new(Position::new(f.start_line, 0), Position::new(f.end_line, 0))),
        )
        .chain(
            features::inlay_hints::inlay_hints(&doc, &index, whole, ENC)
                .iter()
                .map(|h| Range::new(h.position, h.position)),
        )
    {
        assert_applicable(&doc, range, source);
    }

    let _ = features::symbols::document_symbols(&uri, &doc, ENC);
    let _ = features::semantic_tokens::semantic_tokens(&doc, &index, ENC);
    let _ = features::semantic_tokens::semantic_tokens_range(&doc, &index, whole, ENC);
    for edit in features::formatting::formatting(&doc, &options, ENC)
        .into_iter()
        .chain(features::formatting::range_formatting(
            &doc, whole, &options, ENC,
        ))
        .flatten()
    {
        assert_applicable(&doc, edit.range, source);
    }

    // Positional requests, at every byte the cursor can occupy.
    for offset in 0..=source.len() as u32 {
        if !source.is_char_boundary(offset as usize) {
            continue;
        }
        if let Some(hover) = features::hover::hover(&doc, &index, offset, ENC) {
            if let Some(range) = hover.range {
                assert_applicable(&doc, range, source);
            }
        }
        let _ = features::completion::completion(&doc, &index, offset);
        let _ = features::signature_help::signature_help(&doc, &index, offset);
        let _ =
            features::document_highlight::document_highlight(&uri, &doc, &references, offset, ENC);
        let _ = features::rename::prepare_rename(&doc, &index, offset, ENC);
        let position = doc.line_index.position(&doc.text, offset, ENC);
        let _ = features::selection_range::selection_ranges(&doc, &[position], ENC);
    }
}

/// A range an editor could actually apply: forwards, and inside the document.
fn assert_applicable(doc: &Document, range: Range, source: &str) {
    let ordered =
        (range.start.line, range.start.character) <= (range.end.line, range.end.character);
    assert!(ordered, "range runs backwards: {range:?} in {source:?}");
    assert!(
        range.end.line < doc.line_index.line_count(),
        "range is past the last line: {range:?} in {source:?}"
    );
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

#[test]
fn every_feature_survives_the_corpus() {
    for source in SAMPLES {
        exercise(source);
    }
}

#[test]
fn every_feature_survives_a_truncated_file() {
    // What the server is handed on every keystroke of a file being written.
    for source in SAMPLES {
        for end in 0..=source.len() {
            if source.is_char_boundary(end) {
                exercise(&source[..end]);
            }
        }
    }
}

#[test]
fn every_feature_survives_small_mutations() {
    // Delimiters, because they are what actually derails a parser, plus the
    // few characters whose meaning depends entirely on what follows them.
    let alphabet = [
        '{', '}', '(', ')', '[', ']', ';', ',', '"', '\'', '\\', '^', '|', '*', '#', ':', '.', '~',
        '$', '+', '-', '<', '>', '=', 'A', 'a', '1',
    ];
    let mut rng = Rng(0xfeed_face_1234_5678);

    for _ in 0..2_000 {
        let mut mutated = SAMPLES[rng.below(SAMPLES.len())].to_string();
        for _ in 0..1 + rng.below(3) {
            let mut at = rng.below(mutated.len() + 1);
            while !mutated.is_char_boundary(at) {
                at -= 1;
            }
            if at < mutated.len() && rng.below(3) == 0 {
                let mut to = at + 1;
                while !mutated.is_char_boundary(to) {
                    to += 1;
                }
                mutated.replace_range(at..to, "");
            } else {
                mutated.insert(at, alphabet[rng.below(alphabet.len())]);
            }
        }
        exercise(&mutated);
    }
}
