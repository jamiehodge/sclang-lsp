//! Selection ranges: the chain of progressively larger syntactic regions
//! around a position.
//!
//! This is `Shift+Alt+Right` in an editor, and it is also the only
//! parser-backed way to answer "what is the block around the cursor" — the
//! question anything evaluating SuperCollider has to ask first. Counting
//! parentheses answers it wrongly on `"("`, `$(`, `'('` and `// (`; a lossless
//! tree cannot, because the parentheses it reports are the ones the grammar
//! saw.

use crate::documents::Document;
use crate::line_index::PositionEncoding;
use lsp_types::{Position, SelectionRange};
use sclang_syntax::{Child, SyntaxNode};

pub fn selection_ranges(
    doc: &Document,
    positions: &[Position],
    enc: PositionEncoding,
) -> Vec<SelectionRange> {
    positions
        .iter()
        .map(|pos| {
            let offset = doc.line_index.offset(&doc.text, *pos, enc);
            let ranges = ancestors(&doc.parse().root, offset);
            build(doc, &ranges, enc)
        })
        .collect()
}

/// Byte ranges containing `offset`, outermost first.
///
/// Consecutive duplicates are dropped: a CST node that wraps a single child
/// spans exactly the same text, and emitting both would make one press of
/// expand-selection appear to do nothing.
fn ancestors(root: &SyntaxNode, offset: u32) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut node = root;

    loop {
        push(&mut out, (node.start, node.end));

        match descend(node, offset) {
            // Keep walking down the tree.
            Some(Child::Node(child)) => node = child,
            // A leaf ends the chain. Trivia is not worth selecting, so the
            // enclosing node is the innermost useful answer when the cursor
            // sits in whitespace or inside a comment.
            Some(Child::Token(token)) => {
                if !token.kind.is_trivia() {
                    push(&mut out, (token.start, token.end));
                }
                break;
            }
            None => break,
        }
    }

    out
}

fn push(out: &mut Vec<(u32, u32)>, range: (u32, u32)) {
    if out.last() != Some(&range) {
        out.push(range);
    }
}

/// The child of `node` that `offset` falls in.
///
/// A cursor at the end of `foo` is on `foo`, not on whatever follows it, so a
/// child ending exactly at the offset is taken when nothing strictly contains
/// it. That is the position the user is in after typing a name.
fn descend(node: &SyntaxNode, offset: u32) -> Option<&Child> {
    node.children
        .iter()
        .find(|c| {
            let (start, end) = c.range();
            start <= offset && offset < end
        })
        .or_else(|| node.children.iter().rev().find(|c| c.range().1 == offset))
}

fn build(doc: &Document, ranges: &[(u32, u32)], enc: PositionEncoding) -> SelectionRange {
    // Outermost first, so each range becomes the parent of the next.
    let mut built: Option<SelectionRange> = None;

    for (start, end) in ranges {
        built = Some(SelectionRange {
            range: doc.line_index.range(&doc.text, *start..*end, enc),
            parent: built.map(Box::new),
        });
    }

    // `ancestors` always yields the root, so this is only reachable for an
    // empty tree — for which the whole (empty) document is the honest answer.
    built.unwrap_or_else(|| SelectionRange {
        range: doc.line_index.range(&doc.text, 0..0, enc),
        parent: None,
    })
}
