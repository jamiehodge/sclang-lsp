//! Inlay hints: the parameter name a positional argument is filling.
//!
//! `SinOsc.ar(440, 0, 0.5)` says nothing about which of four parameters each
//! number is. The index knows, so the editor can show it.
//!
//! Only for calls whose receiver is a literal class name. An inlay hint is a
//! flat assertion — it renders as though it were in the source — and with an
//! unknown receiver the selector may exist on dozens of classes with different
//! parameter names. Labelling an argument from whichever one happened to be
//! first would be a guess wearing the costume of a fact.

use crate::analysis::{callee, Receiver};
use crate::documents::Document;
use crate::line_index::PositionEncoding;
use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Range};
use sclang_index::{MethodKind, SymbolIndex};
use sclang_syntax::{Child, SyntaxKind, SyntaxNode};

pub fn inlay_hints(
    doc: &Document,
    index: &SymbolIndex,
    range: Range,
    enc: PositionEncoding,
) -> Vec<InlayHint> {
    let root = &doc.parse().root;
    let source = &doc.text;
    // The client asks for one viewport at a time, so only that span is walked.
    let from = doc.line_index.offset(source, range.start, enc);
    let to = doc.line_index.offset(source, range.end, enc);

    let mut out = Vec::new();
    for call in root.descendants() {
        if call.end < from || call.start > to {
            continue;
        }
        if !matches!(call.kind, SyntaxKind::MethodCall | SyntaxKind::CallExpr) {
            continue;
        }
        let Some(arg_list) = call.child_of(SyntaxKind::ArgList) else {
            continue;
        };
        hints_for(call, arg_list, doc, index, enc, &mut out);
    }
    out
}

fn hints_for(
    call: &SyntaxNode,
    arg_list: &SyntaxNode,
    doc: &Document,
    index: &SymbolIndex,
    enc: PositionEncoding,
    out: &mut Vec<InlayHint>,
) {
    let source = &doc.text;
    let Some((selector, receiver)) = callee(call, arg_list, source) else {
        return;
    };
    let Receiver::Class(class) = receiver else {
        return;
    };

    // Same resolution as goto: walk the chain and take the first definition.
    let method = [MethodKind::Class, MethodKind::Instance]
        .into_iter()
        .find_map(|kind| {
            index
                .superclass_chain(&class)
                .into_iter()
                .find_map(|c| index.method(&c.name, &selector, kind))
        });
    let Some(method) = method else { return };

    for (position, argument) in positional_arguments(arg_list).enumerate() {
        let Some(arg) = method.args.get(position) else {
            // More arguments than parameters, or a `...rest` swallowing them.
            return;
        };
        if arg.is_rest {
            return;
        }

        let (start, end) = argument.range();
        // `foo(freq)` needs no hint saying `freq:`.
        if source[start as usize..end as usize].trim() == arg.name {
            continue;
        }

        out.push(InlayHint {
            position: doc.line_index.position(source, start, enc),
            label: InlayHintLabel::String(format!("{}:", arg.name)),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: None,
            padding_left: Some(false),
            padding_right: Some(true),
            data: None,
        });
    }
}

/// The arguments passed by position, in order.
///
/// A `name:` argument is skipped rather than counted: it already says which
/// parameter it fills, and in SuperCollider it does not advance the positional
/// sequence either.
fn positional_arguments(arg_list: &SyntaxNode) -> impl Iterator<Item = &Child> {
    arg_list.children.iter().filter(|child| {
        !child.kind().is_trivia()
            && !matches!(
                child.kind(),
                SyntaxKind::LParen
                    | SyntaxKind::RParen
                    | SyntaxKind::Comma
                    | SyntaxKind::KeywordArg
                    // A trailing block, as in `if (x) { ... }`, reads clearly
                    // enough without a label.
                    | SyntaxKind::FunctionBlock
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn library() -> SymbolIndex {
        let mut index = SymbolIndex::default();
        index.index_file(
            Path::new("lib.sc"),
            "Object { }
             UGen : Object { }
             SinOsc : UGen { *ar { |freq = 440, phase = 0, mul = 1| ^freq } }
             Bag : Object { *new { |...items| ^items } }
             A { blend { |that, amount| ^that } }
             B { blend { |other, ratio| ^other } }",
        );
        index
    }

    fn hints(source: &str) -> Vec<(String, u32)> {
        let doc = Document::new(source.to_string(), 1);
        let whole = Range::new(
            lsp_types::Position::new(0, 0),
            lsp_types::Position::new(u32::MAX, 0),
        );
        inlay_hints(&doc, &library(), whole, PositionEncoding::Utf16)
            .into_iter()
            .map(|h| {
                let label = match h.label {
                    InlayHintLabel::String(s) => s,
                    _ => panic!("expected a string label"),
                };
                (label, h.position.character)
            })
            .collect()
    }

    #[test]
    fn labels_positional_arguments() {
        let got = hints("SinOsc.ar(440, 0, 0.5)");
        let labels: Vec<_> = got.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["freq:", "phase:", "mul:"]);
        // Positioned at each argument, not at the call.
        assert_eq!(got[0].1, 10);
    }

    #[test]
    fn nothing_for_an_unknown_receiver() {
        // `blend` exists on two classes with different parameter names, so
        // any label here would be a guess.
        assert!(hints("x.blend(1, 2)").is_empty());
    }

    #[test]
    fn keyword_arguments_are_left_alone() {
        let got = hints("SinOsc.ar(freq: 440, phase: 0)");
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn no_hint_when_the_argument_already_says_it() {
        let got = hints("SinOsc.ar(freq, 0)");
        let labels: Vec<_> = got.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["phase:"]);
    }

    #[test]
    fn rest_parameters_get_no_hints() {
        assert!(hints("Bag(1, 2, 3)").is_empty());
    }

    #[test]
    fn surplus_arguments_do_not_panic() {
        let got = hints("SinOsc.ar(1, 2, 3, 4, 5)");
        assert_eq!(got.len(), 3, "one per declared parameter: {got:?}");
    }

    #[test]
    fn constructor_syntax_is_labelled_through_new() {
        let got = hints("SinOsc(880)");
        // `SinOsc(...)` is `SinOsc.new(...)`, and nothing declares `*new`
        // here, so there is nothing to label.
        assert!(got.is_empty(), "{got:?}");
    }
}
