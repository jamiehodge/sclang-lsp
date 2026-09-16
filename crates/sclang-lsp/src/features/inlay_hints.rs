//! Inlay hints: the parameter name a positional argument is filling.
//!
//! `SinOsc.ar(440, 0, 0.5)` says nothing about which of four parameters each
//! number is. The index knows, so the editor can show it.
//!
//! Only where the receiver's class is a fact of the grammar and dispatch
//! resolves to one method. An inlay hint is a flat assertion — it renders as
//! though it were in the source — and with an unknown receiver the selector may
//! exist on dozens of classes with different parameter names. Labelling an
//! argument from whichever one happened to be first would be a guess wearing
//! the costume of a fact.
//!
//! A class name is such a fact, and so are `"a"`, `[1, 2]`, `{ }` and `this`:
//! nothing can make them wrong. `Foo.new` is not — a `*new` is free to return
//! something else, and a few classes do — so it is trusted to navigate by and
//! not to write parameter names from.

use crate::analysis::{callee, resolve_selector, ReceiverContext};
use crate::documents::Document;
use crate::line_index::PositionEncoding;
use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Range};
use sclang_index::SymbolIndex;
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
        hints_for(root, call, arg_list, doc, index, enc, &mut out);
    }
    out
}

fn hints_for(
    root: &SyntaxNode,
    call: &SyntaxNode,
    arg_list: &SyntaxNode,
    doc: &Document,
    index: &SymbolIndex,
    enc: PositionEncoding,
    out: &mut Vec<InlayHint>,
) {
    let source = &doc.text;
    // The walk is over descendants, so the context a receiver needs has to be
    // looked up rather than carried down.
    let ctx = ReceiverContext::at(root, source, call.start);
    let Some((selector, receiver)) = callee(call, arg_list, source, &ctx) else {
        return;
    };
    if !receiver.is_certain() {
        return;
    }

    // Same resolution as goto, and only where it narrowed to one definition.
    let Some(method) = resolve_selector(index, &selector, &receiver).single() else {
        return;
    };

    for (position, argument) in positional_arguments(arg_list).into_iter().enumerate() {
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

/// The arguments passed by position, in order: one per comma-separated slot,
/// taken at the expression that opens it.
///
/// Counting every child that is not punctuation was close but not right. An
/// argument is an `exprseq`, so `f(a; b)` passes *one* argument whose value is
/// `b` — counting the halves separately put the second parameter's label on
/// the `;`, and `f(1;)` grew a label for an argument that is not there.
fn positional_arguments(arg_list: &SyntaxNode) -> Vec<&Child> {
    let mut out = Vec::new();
    let mut opening = true;
    for child in &arg_list.children {
        match child.kind() {
            k if k.is_trivia() => {}
            // A new slot begins.
            SyntaxKind::LParen | SyntaxKind::Comma => opening = true,
            // Still inside the slot that is open.
            SyntaxKind::Semicolon | SyntaxKind::RParen => {}
            // `name:` says which parameter it fills, and in SuperCollider it
            // does not advance the positional sequence either.
            SyntaxKind::KeywordArg => opening = false,
            // A trailing block, as in `if (x) { ... }`, reads clearly enough
            // without a label.
            SyntaxKind::FunctionBlock | SyntaxKind::Generator => opening = false,
            // `f(*args)` spreads one array across every remaining parameter.
            // It does not fill the one it sits in front of, and nothing after
            // it has a position worth asserting.
            SyntaxKind::SplatArg => return out,
            _ => {
                if opening {
                    out.push(child);
                    opening = false;
                }
            }
        }
    }
    out
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
    fn a_semicolon_does_not_advance_the_parameter() {
        // `arglist1 : exprseq`, and an `exprseq` may contain `;` — so this
        // passes one argument whose value is `0`. Counting the halves put
        // `phase:` on the semicolon, and `SinOsc.ar(1;)` grew a label for an
        // argument that is not there.
        let got = hints("SinOsc.ar(1; 0, 0.5)");
        let labels: Vec<_> = got.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["freq:", "phase:"], "{got:?}");
        // On the `1`, not on the `;`.
        assert_eq!(got[0].1, 10);
        assert!(hints("SinOsc.ar(1;)").len() == 1);
    }

    #[test]
    fn nothing_is_labelled_past_an_expanded_array() {
        // `*args` spreads across every remaining parameter, so it does not
        // fill the one it sits in front of and nothing after it has a
        // position worth asserting.
        assert!(hints("SinOsc.ar(*args)").is_empty());
        assert!(hints("SinOsc.ar(440, *rest)").len() == 1);
    }

    #[test]
    fn constructor_syntax_is_labelled_through_new() {
        let got = hints("SinOsc(880)");
        // `SinOsc(...)` is `SinOsc.new(...)`, and nothing declares `*new`
        // here, so there is nothing to label.
        assert!(got.is_empty(), "{got:?}");
    }
}
