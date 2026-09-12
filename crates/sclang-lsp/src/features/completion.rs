//! Completion.
//!
//! Two questions decide everything here: what is the user typing, and is the
//! receiver a class name? Only the second admits an exact answer. When it does
//! not, this returns every class that defines the selector rather than
//! guessing — see "Deliberately not done" in ARCHITECTURE.md.

use crate::analysis::{call_at, resolve_selector, token_at, Bias, Receiver};
use crate::documents::Document;
use crate::scope::{locals_at, Local, LocalKind};
use sclang_index::{Method, MethodKind, SymbolIndex};
use sclang_syntax::{Child, SyntaxKind, SyntaxNode};

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, Documentation,
    MarkupContent, MarkupKind,
};

/// How many items to return before giving up and asking the client to come
/// back with a longer prefix.
///
/// A bare `.` on an unknown receiver matches every selector in the class
/// library — some thousands. Truncating with `is_incomplete` set is the
/// protocol's own answer to this: the client re-requests on the next
/// keystroke, so the list narrows as the user types.
const LIMIT: usize = 1000;

/// What the cursor is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// After a `.`, with a possibly-empty prefix typed so far.
    Selector {
        prefix: String,
        receiver: Receiver,
    },
    /// An uppercase-initial word: a class name.
    ClassName {
        prefix: String,
    },
    /// A bare lowercase word. It could be a variable this server does not
    /// track or a unary message send, so selectors are the useful offer.
    Bare {
        prefix: String,
    },
    Nothing,
}

/// Work out what is being completed at `offset`.
pub fn context_at(root: &SyntaxNode, source: &str, offset: u32) -> Context {
    // `Before` is the right bias throughout: the cursor sits after the
    // characters typed so far.
    let Some(path) = token_at(root, offset, Bias::Before) else {
        return Context::Nothing;
    };
    let token = path.token;
    // Only the part of the token to the left of the cursor has been typed.
    let prefix = |token: &sclang_syntax::Token| {
        source[token.start as usize..offset.min(token.end) as usize].to_string()
    };

    match token.kind {
        SyntaxKind::Dot => match path.parent() {
            Some(parent) if parent.kind == SyntaxKind::MethodCall => Context::Selector {
                prefix: String::new(),
                receiver: receiver_before(parent, token.start, source),
            },
            _ => Context::Nothing,
        },
        SyntaxKind::ClassName => Context::ClassName {
            prefix: prefix(&token),
        },
        SyntaxKind::Ident => {
            let dot = path
                .parent()
                .filter(|p| p.kind == SyntaxKind::MethodCall)
                .and_then(|p| {
                    p.child_tokens()
                        .filter(|t| t.kind == SyntaxKind::Dot && t.end <= token.start)
                        .last()
                });
            match (dot, path.parent()) {
                (Some(dot), Some(parent)) => Context::Selector {
                    prefix: prefix(&token),
                    receiver: receiver_before(parent, dot.start, source),
                },
                _ => Context::Bare {
                    prefix: prefix(&token),
                },
            }
        }
        _ => Context::Nothing,
    }
}

/// The receiver node immediately left of an offset within a call.
fn receiver_before(call: &SyntaxNode, before: u32, source: &str) -> Receiver {
    let node = call.children.iter().rev().find(|c| c.range().1 <= before);
    match node {
        Some(Child::Node(n)) if n.kind == SyntaxKind::ClassRef => n
            .token_of(SyntaxKind::ClassName)
            .map(|t| Receiver::Class(t.text(source).to_string()))
            .unwrap_or(Receiver::Unknown),
        _ => Receiver::Unknown,
    }
}

pub fn completion(doc: &Document, index: &SymbolIndex, offset: u32) -> CompletionResponse {
    let context = context_at(&doc.parse().root, &doc.text, offset);
    let mut items = Vec::new();
    let mut truncated = false;

    match context {
        Context::ClassName { prefix } => {
            for class in index.classes_with_prefix(&prefix) {
                if items.len() >= LIMIT {
                    truncated = true;
                    break;
                }
                items.push(class_item(class));
            }
        }
        Context::Selector { prefix, receiver } => match receiver {
            Receiver::Class(name) => {
                // A class object answers its own class-side methods, and also
                // the instance methods of `Class` and `Object` above it —
                // which is why `SinOsc.dumpInterface` works.
                let mut methods = index.methods_visible_on(&name, MethodKind::Class);
                methods.extend(index.methods_visible_on("Class", MethodKind::Instance));
                for m in methods {
                    if !m.name.starts_with(&prefix) {
                        continue;
                    }
                    if items.len() >= LIMIT {
                        truncated = true;
                        break;
                    }
                    items.push(method_item(m, Some(&name)));
                }
            }
            Receiver::Unknown => {
                truncated = push_selectors_by_name(index, &prefix, &mut items);
            }
        },
        Context::Bare { prefix } => {
            push_keyword_args(doc, index, offset, &prefix, &mut items);
            // Names the user wrote themselves come first. Before this, typing
            // `fr` inside a method that declares `freq` offered sixty global
            // selectors and not the one name actually in scope.
            for local in locals_at(&doc.parse().root, &doc.text, offset) {
                if local.name.starts_with(&prefix) {
                    items.push(local_item(&local));
                }
            }
            let before = items.len();
            truncated = push_selectors_by_name(index, &prefix, &mut items);
            // Rank the two groups rather than letting the client interleave
            // them alphabetically, which would bury a local among globals.
            for (i, item) in items.iter_mut().enumerate() {
                if item.sort_text.is_some() {
                    continue; // a keyword argument, already ranked
                }
                item.sort_text = Some(format!("{}{}", if i < before { 1 } else { 2 }, item.label));
            }
        }
        // Directly after `(`, with nothing typed. Not a token the classifier
        // has anything to say about, but the call around it does.
        Context::Nothing => {
            push_keyword_args(doc, index, offset, "", &mut items);
        }
    }

    CompletionResponse::List(CompletionList {
        is_incomplete: truncated,
        items,
    })
}

/// Offer one item per distinct selector name, not one per implementor.
///
/// `postln` is defined on dozens of classes; a user choosing it does not want
/// to choose which class's copy.
fn push_selectors_by_name(
    index: &SymbolIndex,
    prefix: &str,
    items: &mut Vec<CompletionItem>,
) -> bool {
    for name in index.method_names_with_prefix(prefix) {
        if items.len() >= LIMIT {
            return true;
        }
        let implementors = index.implementors(name);
        let Some(first) = implementors.first() else {
            continue;
        };
        let mut item = method_item(first, None);
        if implementors.len() > 1 {
            item.detail = Some(format!(
                "{}  (+{} more)",
                first.signature(),
                implementors.len() - 1
            ));
        }
        items.push(item);
    }
    false
}

/// Offer `name:` for each parameter of the call the cursor is inside.
///
/// Only when the receiver resolves exactly. With an unknown receiver the
/// selector may be defined on dozens of classes with different parameter
/// names, and inventing one set would be a guess dressed as knowledge.
fn push_keyword_args(
    doc: &Document,
    index: &SymbolIndex,
    offset: u32,
    prefix: &str,
    items: &mut Vec<CompletionItem>,
) {
    let Some(call) = call_at(&doc.parse().root, &doc.text, offset) else {
        return;
    };
    if !matches!(call.receiver, Receiver::Class(_)) {
        return;
    }
    let methods = resolve_selector(index, &call.selector, &call.receiver);
    let Some(method) = methods.first() else {
        return;
    };

    for arg in &method.args {
        // A `...rest` parameter cannot be passed by name.
        if arg.is_rest || !arg.name.starts_with(prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: format!("{}:", arg.name),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(match &arg.default {
                Some(default) => format!("{} — default {}", method.signature(), default),
                None => method.signature(),
            }),
            // Sorts above both locals and selectors: inside a call, naming a
            // parameter is usually what was meant.
            sort_text: Some(format!("0{}", arg.name)),
            ..Default::default()
        });
    }
}

fn local_item(local: &Local) -> CompletionItem {
    let detail = match &local.default {
        Some(default) => format!("{} = {}", local.kind.describe(), default),
        None => local.kind.describe().to_string(),
    };
    CompletionItem {
        label: local.name.clone(),
        kind: Some(match local.kind {
            LocalKind::Argument | LocalKind::Variable => CompletionItemKind::VARIABLE,
            LocalKind::InstanceVar | LocalKind::ClassVar => CompletionItemKind::FIELD,
            LocalKind::Constant => CompletionItemKind::CONSTANT,
        }),
        detail: Some(detail),
        ..Default::default()
    }
}

fn method_item(m: &Method, on_class: Option<&str>) -> CompletionItem {
    let detail = match on_class {
        // The defining class is the useful half when it is not the one typed.
        Some(receiver) if receiver != m.owner => {
            format!("{}  — {}", m.signature(), m.owner)
        }
        _ => m.signature(),
    };
    CompletionItem {
        label: m.name.clone(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(detail),
        documentation: m.doc.as_deref().map(markdown),
        ..Default::default()
    }
}

fn class_item(c: &sclang_index::Class) -> CompletionItem {
    CompletionItem {
        label: c.name.clone(),
        kind: Some(CompletionItemKind::CLASS),
        detail: c.superclass.as_ref().map(|s| format!(": {s}")),
        documentation: c.doc.as_deref().map(markdown),
        ..Default::default()
    }
}

fn markdown(text: &str) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sclang_syntax::parse;
    use std::path::Path;

    fn ctx(source: &str, offset: u32) -> Context {
        let parse = parse(source);
        context_at(&parse.root, source, offset)
    }

    fn index_of(files: &[(&str, &str)]) -> SymbolIndex {
        let mut index = SymbolIndex::default();
        for (name, source) in files {
            index.index_file(Path::new(name), source);
        }
        index
    }

    fn labels(r: &CompletionResponse) -> Vec<String> {
        match r {
            CompletionResponse::List(l) => l.items.iter().map(|i| i.label.clone()).collect(),
            CompletionResponse::Array(a) => a.iter().map(|i| i.label.clone()).collect(),
        }
    }

    #[test]
    fn after_a_dot_on_a_class() {
        assert_eq!(
            ctx("SinOsc.", 7),
            Context::Selector {
                prefix: String::new(),
                receiver: Receiver::Class("SinOsc".into()),
            }
        );
    }

    #[test]
    fn partial_selector_keeps_only_what_is_typed() {
        // Cursor between `a` and `r`: the prefix is `a`, not `ar`.
        assert_eq!(
            ctx("SinOsc.ar", 8),
            Context::Selector {
                prefix: "a".into(),
                receiver: Receiver::Class("SinOsc".into()),
            }
        );
    }

    #[test]
    fn partial_class_name() {
        assert_eq!(
            ctx("Sin", 3),
            Context::ClassName {
                prefix: "Sin".into()
            }
        );
    }

    #[test]
    fn class_side_completion_includes_inherited() {
        let index = index_of(&[(
            "lib.sc",
            "Object { }
             UGen : Object { *multiNew { |a| ^a } }
             SinOsc : UGen { *ar { |freq = 440| ^freq } }",
        )]);
        let doc = Document::new("SinOsc.".into(), 1);
        let got = labels(&completion(&doc, &index, 7));
        assert!(got.contains(&"ar".to_string()));
        // Inherited from UGen, which is the point of walking the chain.
        assert!(got.contains(&"multiNew".to_string()));
    }

    #[test]
    fn unknown_receiver_offers_every_selector_once() {
        let index = index_of(&[(
            "lib.sc",
            "A { play { ^1 } } B { play { ^2 } } C { stop { ^3 } }",
        )]);
        let doc = Document::new("x.p".into(), 1);
        let got = labels(&completion(&doc, &index, 3));
        // `play` is defined twice but offered once.
        assert_eq!(got, vec!["play".to_string()]);
    }

    #[test]
    fn class_name_completion_filters_by_prefix() {
        let index = index_of(
            &["lib.sc"]
                .iter()
                .map(|n| (*n, "SinOsc { } Saw { } LFNoise0 { }"))
                .collect::<Vec<_>>(),
        );
        let doc = Document::new("S".into(), 1);
        let got = labels(&completion(&doc, &index, 1));
        assert_eq!(got, vec!["Saw".to_string(), "SinOsc".to_string()]);
    }

    #[test]
    fn no_completion_in_empty_space() {
        let index = index_of(&[("lib.sc", "A { }")]);
        let doc = Document::new("   ".into(), 1);
        assert!(labels(&completion(&doc, &index, 2)).is_empty());
    }

    #[test]
    fn locals_come_before_globals() {
        let index = index_of(&[("lib.sc", "A { frac { ^1 } fragment { ^2 } }")]);
        let doc = Document::new("T { m { |freq = 440| ^fr } }".into(), 1);
        let offset = doc.text.find("^fr").unwrap() as u32 + 3;
        let response = completion(&doc, &index, offset);
        let CompletionResponse::List(list) = response else {
            panic!("expected a list");
        };

        let freq = list
            .items
            .iter()
            .find(|i| i.label == "freq")
            .expect("the argument in scope");
        assert_eq!(freq.detail.as_deref(), Some("argument = 440"));

        // Sorted, the argument must land above the global selectors that
        // merely share its prefix.
        let mut sorted = list.items.clone();
        sorted.sort_by_key(|i| i.sort_text.clone().unwrap_or_else(|| i.label.clone()));
        assert_eq!(sorted[0].label, "freq");
        assert!(sorted.iter().any(|i| i.label == "frac"));
    }

    #[test]
    fn locals_are_not_offered_after_a_dot() {
        // `x.fr` is a message send; a local named `freq` is not a candidate.
        let index = index_of(&[("lib.sc", "A { frac { ^1 } }")]);
        let doc = Document::new("T { m { |freq| ^x.fr } }".into(), 1);
        let offset = doc.text.find("x.fr").unwrap() as u32 + 4;
        let CompletionResponse::List(list) = completion(&doc, &index, offset) else {
            panic!("expected a list");
        };
        let got = list
            .items
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>();
        assert!(got.contains(&"frac"), "{got:?}");
        assert!(!got.contains(&"freq"), "{got:?}");
    }
}
