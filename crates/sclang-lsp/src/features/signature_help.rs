//! Signature help: what the call under the cursor takes.
//!
//! The index has carried argument names and defaults since before there was a
//! server; this only has to find the call, resolve the selector, and count
//! commas. Where the receiver is not a literal class name the selector may
//! belong to many classes, and the protocol's list of signatures is the honest
//! shape for saying so.

use crate::analysis::{ancestors_at, resolve_selector, skip_trivia_back, Receiver};
use crate::documents::Document;
use sclang_index::{Method, SymbolIndex};
use sclang_syntax::{Child, SyntaxKind, SyntaxNode};

use lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

/// How many candidate signatures to offer when the receiver is unknown.
const MAX_SIGNATURES: usize = 8;

pub fn signature_help(doc: &Document, index: &SymbolIndex, offset: u32) -> Option<SignatureHelp> {
    let root = &doc.parse().root;
    let source = &doc.text;
    // A cursor sitting in the space after a comma is still inside the call.
    let offset = skip_trivia_back(root, offset);
    let path = ancestors_at(root, offset);

    // The innermost argument list wins, so a nested call inside an argument
    // describes itself rather than its enclosing call.
    let position = path.iter().rposition(|n| n.kind == SyntaxKind::ArgList)?;
    let arg_list = path[position];
    let call = path.get(position.checked_sub(1)?)?;

    let (name, receiver) = callee(call, arg_list, source)?;
    let methods = resolve_selector(index, &name, &receiver);
    if methods.is_empty() {
        return None;
    }

    let active = active_parameter(arg_list, source, offset, methods[0]);

    let signatures: Vec<_> = methods
        .iter()
        .take(MAX_SIGNATURES)
        .map(|m| signature_of(m))
        .collect();

    Some(SignatureHelp {
        signatures,
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}

/// The selector being called and what it is being sent to.
fn callee(call: &SyntaxNode, arg_list: &SyntaxNode, source: &str) -> Option<(String, Receiver)> {
    match call.kind {
        // `receiver.selector(...)`
        SyntaxKind::MethodCall => {
            let dot = call
                .child_tokens()
                .filter(|t| t.kind == SyntaxKind::Dot && t.end <= arg_list.start)
                .last()?;
            let selector = call
                .child_tokens()
                .find(|t| t.kind == SyntaxKind::Ident && t.start >= dot.end)?;
            let receiver = call
                .children
                .iter()
                .rev()
                .find(|c| c.range().1 <= dot.start);
            let receiver = match receiver {
                Some(Child::Node(n)) if n.kind == SyntaxKind::ClassRef => n
                    .token_of(SyntaxKind::ClassName)
                    .map(|t| Receiver::Class(t.text(source).to_string()))
                    .unwrap_or(Receiver::Unknown),
                _ => Receiver::Unknown,
            };
            Some((selector.text(source).to_string(), receiver))
        }

        SyntaxKind::CallExpr => {
            let head = call.child_nodes().next()?;
            match head.kind {
                // `Point(1, 2)` is `Point.new(1, 2)`.
                SyntaxKind::ClassRef => {
                    let class = head.token_of(SyntaxKind::ClassName)?;
                    Some((
                        "new".to_string(),
                        Receiver::Class(class.text(source).to_string()),
                    ))
                }
                // `foo(a, b)` is `a.foo(b)` — the receiver is the first
                // argument, whose type is unknown, so every implementor of
                // `foo` is a candidate.
                SyntaxKind::NameRef => {
                    let name = head.token_of(SyntaxKind::Ident)?;
                    Some((name.text(source).to_string(), Receiver::Unknown))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Which parameter the cursor sits on.
///
/// Positionally that is the number of commas behind it. A keyword argument
/// overrides that, since `SinOsc.ar(mul: 0.5` is on `mul` whatever its
/// position.
fn active_parameter(arg_list: &SyntaxNode, source: &str, offset: u32, method: &Method) -> u32 {
    if let Some(name) = keyword_at(arg_list, source, offset) {
        if let Some(index) = method.args.iter().position(|a| a.name == name) {
            return index as u32;
        }
    }

    arg_list
        .child_tokens()
        .filter(|t| t.kind == SyntaxKind::Comma && t.end <= offset)
        .count() as u32
}

/// The keyword of the `name:` argument the cursor is inside, if any.
fn keyword_at(arg_list: &SyntaxNode, source: &str, offset: u32) -> Option<String> {
    arg_list
        .child_nodes()
        .filter(|n| n.kind == SyntaxKind::KeywordArg)
        .find(|n| n.start <= offset && offset <= n.end)
        .and_then(|n| n.token_of(SyntaxKind::KeywordBinop))
        .map(|t| t.text(source).trim_end_matches(':').to_string())
}

/// Build one signature, recording where each parameter sits in the label.
///
/// The offsets are UTF-16 units because that is what the protocol counts them
/// in, regardless of what encoding was negotiated for document positions.
fn signature_of(method: &Method) -> SignatureInformation {
    let mut label = String::new();
    label.push_str(&method.owner);
    label.push('.');
    if method.kind == sclang_index::MethodKind::Class {
        label.push('*');
    }
    label.push_str(&method.name);
    label.push('(');

    let mut parameters = Vec::new();
    for (i, arg) in method.args.iter().enumerate() {
        if i > 0 {
            label.push_str(", ");
        }
        let start = utf16_len(&label);
        if arg.is_rest {
            label.push_str("...");
        }
        label.push_str(&arg.name);
        if let Some(default) = &arg.default {
            label.push_str(" = ");
            label.push_str(default);
        }
        let end = utf16_len(&label);

        parameters.push(ParameterInformation {
            label: ParameterLabel::LabelOffsets([start, end]),
            documentation: None,
        });
    }
    label.push(')');

    SignatureInformation {
        label,
        documentation: method.doc.as_ref().map(|d| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: d.clone(),
            })
        }),
        parameters: Some(parameters),
        active_parameter: None,
    }
}

fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn index_of(source: &str) -> SymbolIndex {
        let mut index = SymbolIndex::default();
        index.index_file(Path::new("lib.sc"), source);
        index
    }

    fn library() -> SymbolIndex {
        index_of(
            "Object { }
             UGen : Object { }
             SinOsc : UGen {
                 *ar { |freq = 440, phase = 0, mul = 1, add = 0| ^freq }
             }
             Point : Object { *new { |x = 0, y = 0| ^x } }
             A { blend { |that, amount = 0.5| ^that } }
             B { blend { |other| ^other } }",
        )
    }

    fn help(source: &str, cursor: &str) -> Option<SignatureHelp> {
        let doc = Document::new(source.to_string(), 1);
        let offset = source.find(cursor).expect("cursor marker") as u32 + cursor.len() as u32;
        signature_help(&doc, &library(), offset)
    }

    #[test]
    fn describes_the_call_being_typed() {
        let got = help("SinOsc.ar(", "SinOsc.ar(").expect("signature help");
        assert_eq!(got.signatures.len(), 1);
        assert_eq!(
            got.signatures[0].label,
            "SinOsc.*ar(freq = 440, phase = 0, mul = 1, add = 0)"
        );
        assert_eq!(got.active_parameter, Some(0));
    }

    #[test]
    fn commas_advance_the_active_parameter() {
        assert_eq!(
            help("SinOsc.ar(440, ", "440, ").unwrap().active_parameter,
            Some(1)
        );
        assert_eq!(
            help("SinOsc.ar(440, 0, ", "440, 0, ")
                .unwrap()
                .active_parameter,
            Some(2)
        );
    }

    #[test]
    fn a_keyword_argument_selects_its_own_parameter() {
        // Third parameter by name, first by position.
        let got = help("SinOsc.ar(mul: ", "mul: ").unwrap();
        assert_eq!(got.active_parameter, Some(2));
    }

    #[test]
    fn parameter_offsets_point_at_the_label() {
        let got = help("SinOsc.ar(", "SinOsc.ar(").unwrap();
        let label = &got.signatures[0].label;
        let params = got.signatures[0].parameters.as_ref().unwrap();
        let slice = |i: usize| match params[i].label {
            ParameterLabel::LabelOffsets([a, b]) => &label[a as usize..b as usize],
            _ => panic!("expected offsets"),
        };
        assert_eq!(slice(0), "freq = 440");
        assert_eq!(slice(3), "add = 0");
    }

    #[test]
    fn constructor_syntax_resolves_to_new() {
        let got = help("Point(1, ", "Point(1, ").unwrap();
        assert_eq!(got.signatures[0].label, "Point.*new(x = 0, y = 0)");
        assert_eq!(got.active_parameter, Some(1));
    }

    #[test]
    fn an_unknown_receiver_offers_every_implementor() {
        let got = help("x.blend(", "x.blend(").unwrap();
        assert_eq!(got.signatures.len(), 2);
        let labels: Vec<_> = got.signatures.iter().map(|s| s.label.as_str()).collect();
        assert!(
            labels.contains(&"A.blend(that, amount = 0.5)"),
            "{labels:?}"
        );
        assert!(labels.contains(&"B.blend(other)"), "{labels:?}");
    }

    #[test]
    fn a_nested_call_describes_itself() {
        // The cursor is inside Point(...), not inside SinOsc.ar(...).
        let got = help("SinOsc.ar(Point(1, ", "Point(1, ").unwrap();
        assert_eq!(got.signatures[0].label, "Point.*new(x = 0, y = 0)");
    }

    #[test]
    fn nothing_outside_a_call() {
        assert!(help("SinOsc.ar", "SinOsc.ar").is_none());
        assert!(help("var x = 1;", "var x").is_none());
    }

    #[test]
    fn nothing_for_an_unknown_selector() {
        assert!(help("x.noSuchMethodAnywhere(", "x.noSuchMethodAnywhere(").is_none());
    }
}
