//! Syntax diagnostics, straight from the parser.
//!
//! These are the first of the three sources ARCHITECTURE.md lists. They cost
//! nothing extra — the tree is already built for everything else — and they
//! are available per keystroke, on a file that has never been saved, with no
//! sclang running.

use crate::documents::Document;
use crate::line_index::PositionEncoding;
use lsp_types::{Diagnostic, DiagnosticSeverity};

/// The source string shown against each diagnostic in the editor.
const SOURCE: &str = "sclang-syntax";

pub fn diagnostics(doc: &Document, enc: PositionEncoding) -> Vec<Diagnostic> {
    doc.parse()
        .errors
        .iter()
        .map(|e| {
            let (start, end) = widen(&doc.text, e.start, e.end);
            Diagnostic {
                range: doc.line_index.range(&doc.text, start..end, enc),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(SOURCE.to_string()),
                message: e.message.clone(),
                ..Default::default()
            }
        })
        .collect()
}

/// Give a zero-width error something to underline.
///
/// "expected a method name after '.'" is reported at the point where the name
/// should have been, which is frequently the end of the file — and an empty
/// range draws nothing in most editors. Widen right if there is a character
/// there, otherwise left over the one that prompted the error.
fn widen(text: &str, start: u32, end: u32) -> (u32, u32) {
    if end > start {
        return (start, end);
    }
    if let Some(c) = text[end as usize..].chars().next() {
        return (start, end + c.len_utf8() as u32);
    }
    match text[..start as usize].chars().next_back() {
        Some(c) => (start - c.len_utf8() as u32, end),
        // An empty document, which has nothing to point at anyway.
        None => (start, end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_file_has_no_diagnostics() {
        let doc = Document::new("Foo : Bar { baz { ^1 } }".into(), 1);
        assert!(diagnostics(&doc, PositionEncoding::Utf16).is_empty());
    }

    #[test]
    fn reports_a_syntax_error_with_a_visible_range() {
        let doc = Document::new("SinOsc.".into(), 1);
        let found = diagnostics(&doc, PositionEncoding::Utf16);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Some(DiagnosticSeverity::ERROR));
        // Never zero-width, or the editor draws nothing.
        assert_ne!(found[0].range.start, found[0].range.end);
    }

    #[test]
    fn recovers_and_keeps_reporting_after_an_error() {
        // The point of error recovery: a broken method does not hide the file.
        let doc = Document::new("Foo { bar { ^( } baz { ^1 } }".into(), 1);
        assert!(!diagnostics(&doc, PositionEncoding::Utf16).is_empty());
    }
}
