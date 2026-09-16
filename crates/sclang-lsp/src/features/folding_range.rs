//! Folding ranges.
//!
//! The editor's fallback is indentation, and SuperCollider's central idiom
//! defeats it. A region is `(` and `)` at the start of a line, and what sits
//! between them is routinely not indented at all:
//!
//! ```text
//! (
//! Pdef(\kick,
//!     Pbind(\instrument, \kick)
//! )
//! )
//! ```
//!
//! There is no indentation there to fold on, so the one construct a
//! SuperCollider file is organised around is the one the editor cannot
//! collapse. The parser knows exactly where each region ends.
//!
//! Ranges are line-based. A client may ask for character precision, but VS
//! Code and most others set `lineFoldingOnly`, and a fold that stops mid-line
//! is not a thing anyone wants from a `( … )` block.

use crate::analysis::visit_tokens;
use crate::documents::Document;
use lsp_types::{FoldingRange, FoldingRangeKind};
use sclang_syntax::SyntaxKind;

pub fn folding_ranges(doc: &Document) -> Vec<FoldingRange> {
    let root = &doc.parse().root;
    let source = &doc.text;
    let index = &doc.line_index;
    let mut out: Vec<FoldingRange> = Vec::new();

    for node in root.descendants() {
        if !folds(node.kind) || node.end <= node.start {
            continue;
        }
        // `end` is exclusive, so the last byte is what names the closing line.
        push(
            &mut out,
            index.line(node.start),
            index.line(node.end - 1),
            None,
        );
    }

    let mut region_starts: Vec<u32> = Vec::new();
    visit_tokens(root, &mut |token| {
        match token.kind {
            SyntaxKind::BlockComment => {
                push(
                    &mut out,
                    index.line(token.start),
                    index.line(token.end - 1),
                    Some(FoldingRangeKind::Comment),
                );
            }
            // `//#region` and `//#endregion`, the markers
            // `language-configuration.json` declares. Declaring a folding
            // range provider is what stops the editor handling those itself,
            // so they have to be honoured here or they quietly stop working.
            //
            // Read off the token rather than off the line, so a `//` inside a
            // string is not mistaken for one.
            SyntaxKind::LineComment => {
                if !starts_the_line(source, token.start) {
                    return;
                }
                match marker(token.text(source)) {
                    Some(Marker::Start) => region_starts.push(index.line(token.start)),
                    Some(Marker::End) => {
                        // Unbalanced markers are ordinary in a file being
                        // edited; an `endregion` with nothing open is simply
                        // not a range yet.
                        if let Some(start) = region_starts.pop() {
                            push(
                                &mut out,
                                start,
                                index.line(token.start),
                                Some(FoldingRangeKind::Region),
                            );
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    });

    out.sort_by_key(|r| (r.start_line, r.end_line));
    out
}

/// Nodes worth collapsing: the ones that hold a body rather than merely wrap
/// one. An argument list is deliberately absent — folding `SynthDef(…)` and
/// the function inside it separately gives two chevrons for one thing.
fn folds(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ClassDef
            | SyntaxKind::ClassExtension
            | SyntaxKind::MethodDef
            | SyntaxKind::FunctionBlock
            | SyntaxKind::ParenExpr
            | SyntaxKind::EventLiteral
            | SyntaxKind::ArithSeries
            | SyntaxKind::Collection
            | SyntaxKind::LiteralList
            | SyntaxKind::Generator
    )
}

/// Record a range, if it is one.
///
/// Anything on a single line cannot be folded, and two nodes spanning the same
/// lines — a `( … )` holding nothing but a `[ … ]` — would give the reader two
/// chevrons that do the same thing.
fn push(
    out: &mut Vec<FoldingRange>,
    start_line: u32,
    end_line: u32,
    kind: Option<FoldingRangeKind>,
) {
    if start_line >= end_line {
        return;
    }
    if out
        .iter()
        .any(|r| r.start_line == start_line && r.end_line == end_line)
    {
        return;
    }
    out.push(FoldingRange {
        start_line,
        end_line,
        // Omitted deliberately: see the note on line-based ranges above.
        start_character: None,
        end_character: None,
        kind,
        collapsed_text: None,
    });
}

/// Whether only whitespace precedes this offset on its line.
fn starts_the_line(source: &str, offset: u32) -> bool {
    source[..offset as usize]
        .rsplit('\n')
        .next()
        .is_some_and(|before| before.chars().all(char::is_whitespace))
}

enum Marker {
    Start,
    End,
}

/// `// #region` / `// #endregion`, in the spellings
/// `language-configuration.json` accepts: the `#` is optional and the space
/// is too.
fn marker(comment: &str) -> Option<Marker> {
    let rest = comment.strip_prefix("//")?.trim_start();
    let rest = rest.strip_prefix('#').unwrap_or(rest);

    if let Some(after) = rest.strip_prefix("endregion") {
        ends_the_word(after).then_some(Marker::End)
    } else if let Some(after) = rest.strip_prefix("region") {
        ends_the_word(after).then_some(Marker::Start)
    } else {
        None
    }
}

/// The `\b` the declared patterns end with: `//#regional` is not a marker.
fn ends_the_word(rest: &str) -> bool {
    rest.chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_index::PositionEncoding;

    /// Each range as `(start, end, kind)`, with the kind spelled out so the
    /// expectations read like what a client is told.
    fn folds_of(source: &str) -> Vec<(u32, u32, &'static str)> {
        let doc = Document::with_mode(source.to_string(), 1, sclang_syntax::Mode::Script);
        folding_ranges(&doc)
            .into_iter()
            .map(|r| {
                let kind = match r.kind {
                    Some(FoldingRangeKind::Comment) => "comment",
                    Some(FoldingRangeKind::Region) => "region",
                    Some(FoldingRangeKind::Imports) => "imports",
                    None => "",
                };
                (r.start_line, r.end_line, kind)
            })
            .collect()
    }

    #[test]
    fn a_region_folds_even_with_nothing_indented() {
        // The case indentation folding cannot see: every line starts at
        // column 0, so there is no shape to infer a fold from.
        let source = "(\nPdef(\\kick,\n\tPbind(\\instrument, \\kick)\n)\n)\n";
        let folds = folds_of(source);
        assert!(
            folds.contains(&(0, 4, "")),
            "the outer region should fold: {folds:?}"
        );
    }

    #[test]
    fn a_class_and_its_methods_fold_separately() {
        let source = "Foo : Bar {\n\tbaz {\n\t\t^1\n\t}\n}\n";
        let doc = Document::new(source.to_string(), 1);
        let folds: Vec<_> = folding_ranges(&doc)
            .into_iter()
            .map(|r| (r.start_line, r.end_line))
            .collect();
        assert_eq!(folds, vec![(0, 4), (1, 3)]);
    }

    #[test]
    fn nothing_on_one_line_folds() {
        assert!(folds_of("SinOsc.ar(440);\n").is_empty());
        assert!(folds_of("Foo { bar { ^1 } }\n").is_empty());
    }

    #[test]
    fn a_block_comment_folds_as_a_comment() {
        let folds = folds_of("/* one\n   two\n   three */\n1\n");
        assert_eq!(folds, vec![(0, 2, "comment")]);
    }

    #[test]
    fn region_markers_are_honoured() {
        // Declaring a folding range provider is what stops the editor
        // handling these itself, so they have to work here.
        let folds = folds_of("//#region setup\n1;\n2;\n//#endregion\n");
        assert_eq!(folds, vec![(0, 3, "region")]);
    }

    #[test]
    fn region_markers_take_the_spellings_the_configuration_declares() {
        for start in ["//#region", "// #region", "//region", "// region a"] {
            let source = format!("{start}\n1;\n// endregion\n");
            assert_eq!(
                folds_of(&source),
                vec![(0, 2, "region")],
                "{start:?} should open a region"
            );
        }
        // `\b` at the end of the declared pattern: this is a plain comment.
        assert!(folds_of("//#regional\n1;\n//#endregion\n").is_empty());
    }

    #[test]
    fn a_marker_inside_a_string_is_not_one() {
        // The whole reason to read these off tokens rather than off lines.
        let source = "x = \"\n//#region\n\";\n1;\n";
        let folds = folds_of(source);
        assert!(
            !folds.iter().any(|(_, _, kind)| *kind == "region"),
            "{folds:?}"
        );
    }

    #[test]
    fn an_unmatched_marker_is_not_a_range() {
        // Ordinary while a file is being written.
        assert!(folds_of("//#endregion\n1;\n").is_empty());
        assert!(folds_of("//#region\n1;\n").is_empty());
    }

    #[test]
    fn nested_nodes_spanning_the_same_lines_fold_once() {
        // `( [ … ] )` is two nodes and one chevron.
        let folds = folds_of("([\n\t1,\n\t2\n])\n");
        assert_eq!(folds.len(), 1, "{folds:?}");
    }

    #[test]
    fn ranges_come_out_in_order() {
        let source = "(\n1;\n)\n\n(\n2;\n)\n";
        let folds = folds_of(source);
        let mut sorted = folds.clone();
        sorted.sort();
        assert_eq!(folds, sorted);
    }

    #[test]
    fn a_half_typed_file_still_folds_what_it_can() {
        let doc = Document::new("Foo : Bar {\n\tbaz {\n\t\t^1.".to_string(), 1);
        // No panic, and the class is still a range.
        assert!(!folding_ranges(&doc).is_empty());
    }

    #[test]
    fn lines_do_not_depend_on_the_encoding() {
        // The whole reason this feature negotiates nothing.
        let source = "(\n\"🎛 °ø\";\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, sclang_syntax::Mode::Script);
        let folds = folding_ranges(&doc);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 2));
        // And the index agrees from either direction.
        let index = &doc.line_index;
        assert_eq!(
            index.line(source.find(')').unwrap() as u32),
            index
                .position(
                    source,
                    source.find(')').unwrap() as u32,
                    PositionEncoding::Utf16
                )
                .line
        );
    }
}
