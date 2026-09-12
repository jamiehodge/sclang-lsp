//! Document and workspace symbols.
//!
//! Document symbols come from re-reading the open buffer, so they track
//! unsaved edits. Workspace symbols come from the index.

use crate::documents::{uri_to_path, Document};
use crate::line_index::PositionEncoding;
use crate::locations::Resolver;
use lsp_types::{DocumentSymbol, SymbolInformation, SymbolKind, Url};
use sclang_index::{symbols_of, MethodKind, SymbolIndex};

/// Cap on workspace symbol results. The query is a substring match over some
/// twelve thousand methods, and a client showing a picker wants a page.
const MAX_WORKSPACE_SYMBOLS: usize = 256;

#[allow(deprecated)] // `DocumentSymbol::deprecated` is required but obsolete.
pub fn document_symbols(uri: &Url, doc: &Document, enc: PositionEncoding) -> Vec<DocumentSymbol> {
    let path = uri_to_path(uri);
    let syms = symbols_of(&path, &doc.text);
    let range = |r: &std::ops::Range<u32>| doc.line_index.range(&doc.text, r.clone(), enc);

    let mut out = Vec::new();
    let mut nested = Vec::new();

    for class in &syms.classes {
        let children: Vec<_> = syms
            .methods
            .iter()
            .filter(|m| {
                // Same name *and* inside the class body: a `+ Foo` extension
                // in the same file names the same owner but belongs outside.
                m.owner == class.name
                    && m.location.range.start >= class.location.range.start
                    && m.location.range.end <= class.location.range.end
            })
            .map(|m| {
                nested.push(m.location.range.clone());
                DocumentSymbol {
                    name: m.name.clone(),
                    detail: Some(m.signature()),
                    kind: SymbolKind::METHOD,
                    tags: None,
                    deprecated: None,
                    range: range(&m.location.range),
                    selection_range: range(&m.location.name_range),
                    children: None,
                }
            })
            .collect();

        out.push(DocumentSymbol {
            name: class.name.clone(),
            detail: class.superclass.as_ref().map(|s| format!(": {s}")),
            kind: SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            range: range(&class.location.range),
            selection_range: range(&class.location.name_range),
            children: Some(children),
        });
    }

    // Methods added by `+ Foo { ... }` have no class body in this file, so
    // they belong at the top level rather than being dropped.
    for m in &syms.methods {
        if nested.contains(&m.location.range) {
            continue;
        }
        out.push(DocumentSymbol {
            name: format!("{}.{}", m.owner, m.name),
            detail: Some(m.signature()),
            kind: SymbolKind::METHOD,
            tags: None,
            deprecated: None,
            range: range(&m.location.range),
            selection_range: range(&m.location.name_range),
            children: None,
        });
    }

    out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    out
}

#[allow(deprecated)] // `SymbolInformation::deprecated` is required but obsolete.
pub fn workspace_symbols(
    index: &SymbolIndex,
    query: &str,
    resolver: &Resolver<'_>,
) -> Vec<SymbolInformation> {
    let needle = query.to_lowercase();
    let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
    let mut out = Vec::new();

    for class in index.classes() {
        if out.len() >= MAX_WORKSPACE_SYMBOLS {
            return out;
        }
        if !matches(&class.name) {
            continue;
        }
        if let Some(location) = resolver.resolve(&class.location) {
            out.push(SymbolInformation {
                name: class.name.clone(),
                kind: SymbolKind::CLASS,
                tags: None,
                deprecated: None,
                location,
                container_name: class.superclass.clone(),
            });
        }
    }

    for method in index.methods() {
        if out.len() >= MAX_WORKSPACE_SYMBOLS {
            return out;
        }
        if !matches(&method.name) {
            continue;
        }
        if let Some(location) = resolver.resolve(&method.location) {
            out.push(SymbolInformation {
                name: match method.kind {
                    MethodKind::Class => format!("*{}", method.name),
                    MethodKind::Instance => method.name.clone(),
                },
                kind: SymbolKind::METHOD,
                tags: None,
                deprecated: None,
                location,
                container_name: Some(method.owner.clone()),
            });
        }
    }

    out
}
