//! Find references.
//!
//! Three kinds of answer, with three different standards of proof:
//!
//! * A local binding is exact. Its uses cannot leave the function, and
//!   shadowing is decided by re-resolving each candidate against the scope at
//!   its own position.
//! * A class name is exact too. Only class names lex as `ClassName`, so a
//!   matching token is a reference and a `\Foo` or `"Foo"` is not.
//! * A selector is *textual*, and deliberately so. Dispatch is dynamic, so
//!   every `.play` in the workspace is a candidate for any class's `play` and
//!   there is no honest way to narrow it without types.

use crate::analysis::{point_at, visit_tokens, Bias, Point};
use crate::documents::Document;
use crate::locations::Resolver;
use crate::references::{OccurrenceKind, ReferenceIndex};
use crate::scope::locals_at;
use lsp_types::{Location, Url};
use sclang_syntax::SyntaxKind;

pub fn references(
    uri: &Url,
    doc: &Document,
    refs: &ReferenceIndex,
    offset: u32,
    include_declaration: bool,
    resolver: &Resolver<'_>,
) -> Option<Vec<Location>> {
    let root = &doc.parse().root;
    let source = &doc.text;

    let locations = match point_at(root, source, offset, Bias::Inside) {
        Point::Local { name } => {
            let ranges = local_references(doc, offset, &name, include_declaration);
            let file = uri.to_file_path().ok();
            match file {
                Some(file) => resolver.at_many(ranges.into_iter().map(|r| (file.as_path(), r))),
                // An untitled buffer has no path, but its ranges are still
                // meaningful against itself.
                None => ranges
                    .into_iter()
                    .map(|r| Location {
                        uri: uri.clone(),
                        range: doc.line_index.range(source, r, resolver.encoding()),
                    })
                    .collect(),
            }
        }

        Point::ClassName(name) => {
            let found = refs.find(&name, |kind| match kind {
                OccurrenceKind::ClassDefinition => include_declaration,
                OccurrenceKind::Class => true,
                OccurrenceKind::MethodDefinition | OccurrenceKind::MethodReference => false,
            });
            resolver.at_many(found.into_iter().map(|(p, o)| (p, o.range.clone())))
        }

        Point::Selector { name, .. } | Point::MethodName { name, .. } => {
            let found = refs.find(&name, |kind| match kind {
                OccurrenceKind::MethodDefinition => include_declaration,
                OccurrenceKind::MethodReference => true,
                OccurrenceKind::ClassDefinition | OccurrenceKind::Class => false,
            });
            resolver.at_many(found.into_iter().map(|(p, o)| (p, o.range.clone())))
        }

        Point::Nothing => return None,
    };

    (!locations.is_empty()).then_some(locations)
}

/// Every use of one local binding, by re-resolving each candidate.
///
/// Matching on name alone would sweep up a shadowing declaration in a nested
/// block and a selector that happens to share the spelling. Asking the scope
/// at each candidate's own position what that name means there costs a walk
/// per candidate and gets both right.
pub fn local_references(
    doc: &Document,
    offset: u32,
    name: &str,
    include_declaration: bool,
) -> Vec<std::ops::Range<u32>> {
    let root = &doc.parse().root;
    let source = &doc.text;

    let Some(target) = locals_at(root, source, offset)
        .into_iter()
        .find(|l| l.name == name)
    else {
        return Vec::new();
    };
    // A pseudo-variable is bound by the compiler; there is no declaration and
    // nothing sensible to enumerate.
    if target.name_range.end == target.name_range.start {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    visit_tokens(root, &mut |token| {
        if token.kind == SyntaxKind::Ident && token.text(source) == name {
            candidates.push(token.start..token.end);
        }
    });

    candidates
        .into_iter()
        .filter(|range| {
            // Only where the name is being used as a binding — not as the
            // selector of a message that happens to share its spelling.
            if !matches!(
                point_at(root, source, range.start, Bias::Inside),
                Point::Local { .. }
            ) {
                return false;
            }
            if !include_declaration && *range == target.name_range {
                return false;
            }
            // The binding in force here must be the one asked about.
            locals_at(root, source, range.start)
                .into_iter()
                .find(|l| l.name == name)
                .is_some_and(|l| l.name_range == target.name_range)
        })
        .collect()
}
