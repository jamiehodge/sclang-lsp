//! Goto definition.
//!
//! A class name resolves to exactly one place. A selector resolves to one
//! place only when the receiver is a literal class name; otherwise it resolves
//! to every implementor, and the protocol's array response is the right shape
//! for saying so.

use crate::analysis::{point_at, resolve_selector, Bias, Point};
use crate::documents::Document;
use crate::locations::Resolver;
use crate::scope::locals_at;
use lsp_types::{GotoDefinitionResponse, Url};
use sclang_index::SymbolIndex;

/// How many implementors to offer for an unresolvable selector.
///
/// `value` has hundreds. Past a point a picker stops being navigation and
/// starts being a list, and the user is better served by find-references.
const MAX_IMPLEMENTORS: usize = 50;

pub fn goto_definition(
    uri: &Url,
    doc: &Document,
    index: &SymbolIndex,
    offset: u32,
    resolver: &Resolver<'_>,
) -> Option<GotoDefinitionResponse> {
    let point = point_at(&doc.parse().root, &doc.text, offset, Bias::Inside);

    let locations: Vec<_> = match point {
        Point::ClassName(name) => index
            .class(&name)
            .and_then(|c| resolver.resolve(&c.location))
            .into_iter()
            .collect(),

        Point::Selector { name, receiver } => resolve_selector(index, &name, &receiver)
            .into_iter()
            .take(MAX_IMPLEMENTORS)
            .filter_map(|m| resolver.resolve(&m.location))
            .collect(),

        // A local never leaves the buffer it is written in, so the whole
        // answer is in the tree already in hand.
        Point::Local { name } => locals_at(&doc.parse().root, &doc.text, offset)
            .into_iter()
            // Innermost first, so the first match is the binding in force.
            .find(|l| l.name == name)
            // A pseudo-variable like `this` is bound by the compiler and
            // written down nowhere, so it has an empty range and nothing to
            // jump to.
            .filter(|l| l.name_range.end > l.name_range.start)
            .map(|l| resolver.in_document(uri, doc, l.name_range))
            .into_iter()
            .collect(),

        // The cursor is already on the definition. Offering to jump to it is
        // noise, so say nothing.
        Point::MethodName { .. } | Point::Nothing => Vec::new(),
    };

    match locations.len() {
        0 => None,
        1 => Some(GotoDefinitionResponse::Scalar(
            locations.into_iter().next().unwrap(),
        )),
        _ => Some(GotoDefinitionResponse::Array(locations)),
    }
}
