//! Hover.
//!
//! What can be said without a running image: a class and its superclass
//! chain, a method's signature and the comment above it. SCDoc rendering is
//! tier 2 and deliberately absent — see ARCHITECTURE.md.

use crate::analysis::{point_at, resolve_selector, Bias, Point, Receiver};
use crate::documents::Document;
use crate::line_index::PositionEncoding;
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};
use sclang_index::{Method, MethodKind, SymbolIndex};
use std::fmt::Write;

/// How many implementors to describe when the receiver is unknown.
const MAX_IMPLEMENTORS: usize = 5;

pub fn hover(
    doc: &Document,
    index: &SymbolIndex,
    offset: u32,
    enc: PositionEncoding,
) -> Option<Hover> {
    let root = &doc.parse().root;
    let point = point_at(root, &doc.text, offset, Bias::Inside);
    let value = match point {
        Point::ClassName(name) => class_hover(index, &name)?,
        Point::Selector { name, receiver } => selector_hover(index, &name, &receiver)?,
        Point::MethodName { name, owner } => {
            let owner = owner?;
            let method = index
                .method(&owner, &name, MethodKind::Instance)
                .or_else(|| index.method(&owner, &name, MethodKind::Class))?;
            method_hover(method)
        }
        Point::Nothing => return None,
    };

    // The range is what the editor highlights. Taking it from the token keeps
    // the highlight on the word rather than the whole expression.
    let range = crate::analysis::token_at(root, offset, Bias::Inside).map(|p| {
        doc.line_index
            .range(&doc.text, p.token.start..p.token.end, enc)
    });

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    })
}

fn class_hover(index: &SymbolIndex, name: &str) -> Option<String> {
    let class = index.class(name)?;
    let mut out = String::new();

    let _ = write!(out, "```supercollider\n{}", class.name);
    if let Some(slot) = &class.indexed_slot {
        let _ = write!(out, "[{slot}]");
    }
    if let Some(sup) = &class.superclass {
        let _ = write!(out, " : {sup}");
    }
    out.push_str("\n```");

    // The chain is the single most useful thing to know about a SuperCollider
    // class, and it is exactly what a static index can produce.
    let chain: Vec<_> = index
        .superclass_chain(name)
        .into_iter()
        .skip(1)
        .map(|c| c.name.as_str())
        .collect();
    if !chain.is_empty() {
        let _ = write!(out, "\n\n{}", chain.join(" → "));
    }

    if let Some(doc) = &class.doc {
        let _ = write!(out, "\n\n---\n{doc}");
    }
    Some(out)
}

fn selector_hover(index: &SymbolIndex, name: &str, receiver: &Receiver) -> Option<String> {
    let methods = resolve_selector(index, name, receiver);
    if methods.is_empty() {
        return None;
    }

    if let Receiver::Class(_) = receiver {
        return Some(method_hover(methods[0]));
    }

    // No receiver type, so describe the candidates rather than pretending to
    // have picked one.
    let mut out = String::new();
    let shown = methods.len().min(MAX_IMPLEMENTORS);
    for m in methods.iter().take(shown) {
        let _ = write!(
            out,
            "```supercollider\n{}.{}\n```\n",
            m.owner,
            m.signature()
        );
    }
    if methods.len() > shown {
        let _ = write!(out, "\n*and {} more*", methods.len() - shown);
    }
    if let Some(doc) = methods[0].doc.as_ref() {
        let _ = write!(out, "\n\n---\n{doc}");
    }
    Some(out)
}

fn method_hover(m: &Method) -> String {
    let mut out = format!("```supercollider\n{}.{}\n```", m.owner, m.signature());
    if m.origin == sclang_index::Origin::Accessor {
        // Worth saying: there is no method body to look at.
        out.push_str("\n\n*generated accessor*");
    }
    if let Some(doc) = &m.doc {
        let _ = write!(out, "\n\n---\n{doc}");
    }
    out
}
