//! Document highlight: the other places this name is written, in this file.
//!
//! The same question find-references answers, asked about one buffer — so the
//! answers come from the same two places, and carry the same three standards
//! of proof. What changes is what a wrong answer costs. A highlight is a tint
//! on a word the reader can already see, so the textual answer for a selector
//! is worth giving here even though it is too weak to rename on.
//!
//! `Write` marks where the name is introduced and `Read` where it is used.
//! Assignment is not distinguished from reading: `x = 1` writes to `x`, and
//! saying so would mean deciding what counts as a write for an instance
//! variable behind a generated setter. The declaration is the part the tree
//! settles outright.

use crate::analysis::{point_at, Bias, Point};
use crate::documents::{uri_to_path, Document};
use crate::features::find_references::local_references;
use crate::line_index::PositionEncoding;
use crate::references::{Occurrence, OccurrenceKind, ReferenceIndex};
use crate::scope::locals_at;
use lsp_types::{DocumentHighlight, DocumentHighlightKind, Url};
use std::ops::Range;

pub fn document_highlight(
    uri: &Url,
    doc: &Document,
    refs: &ReferenceIndex,
    offset: u32,
    enc: PositionEncoding,
) -> Option<Vec<DocumentHighlight>> {
    let root = &doc.parse().root;
    let source = &doc.text;

    let mark = |range: Range<u32>, kind: DocumentHighlightKind| DocumentHighlight {
        range: doc.line_index.range(source, range, enc),
        kind: Some(kind),
    };

    // The index is keyed by path, and an open buffer is re-walked on every
    // keystroke, so this is current even for a file that has never been saved.
    let from_index = |name: &str, accept: fn(OccurrenceKind) -> bool| {
        refs.in_file(&uri_to_path(uri), name, accept)
            .into_iter()
            .map(|o: &Occurrence| mark(o.range.clone(), written(o.kind)))
            .collect::<Vec<_>>()
    };

    let highlights = match point_at(root, source, offset, Bias::Inside) {
        Point::Local { name } => {
            // Exact: the uses of a binding cannot leave its function, and
            // `local_references` re-resolves each candidate against the scope
            // at its own position so a shadowing declaration is not swept in.
            let declared = locals_at(root, source, offset)
                .into_iter()
                .find(|l| l.name == name)
                .map(|l| l.name_range);

            local_references(doc, offset, &name, true)
                .into_iter()
                .map(|range| {
                    let kind = if Some(&range) == declared.as_ref() {
                        DocumentHighlightKind::WRITE
                    } else {
                        DocumentHighlightKind::READ
                    };
                    mark(range, kind)
                })
                .collect()
        }

        // Exact: only a class name lexes as one, so `\Foo` and `"Foo"` are
        // left alone.
        Point::ClassName(name) => from_index(&name, |kind| {
            matches!(
                kind,
                OccurrenceKind::ClassDefinition | OccurrenceKind::Class
            )
        }),

        // Textual, and honestly so. Dispatch is dynamic, so every `.play` in
        // the file is a candidate for any class's `play`.
        Point::Selector { name, .. } | Point::MethodName { name, .. } => {
            from_index(&name, |kind| {
                matches!(
                    kind,
                    OccurrenceKind::MethodDefinition | OccurrenceKind::MethodReference
                )
            })
        }

        Point::Nothing => return None,
    };

    (!highlights.is_empty()).then_some(highlights)
}

/// Where a name is introduced rather than used.
fn written(kind: OccurrenceKind) -> DocumentHighlightKind {
    match kind {
        OccurrenceKind::ClassDefinition | OccurrenceKind::MethodDefinition => {
            DocumentHighlightKind::WRITE
        }
        OccurrenceKind::Class | OccurrenceKind::MethodReference => DocumentHighlightKind::READ,
    }
}
