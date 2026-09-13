//! The documents the editor has open, and their text.
//!
//! The server owns document text outright. That is the inversion the whole
//! design rests on: nothing else is consulted about what a buffer contains, so
//! analysis works on unsaved edits, on files that do not compile, and with no
//! other process running.

use crate::line_index::{LineIndex, PositionEncoding};
use lsp_types::{TextDocumentContentChangeEvent, Url};
use sclang_syntax::{parse, Parse};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One open buffer: its text, the revision the editor gave it, and the
/// derived data kept in step with both.
#[derive(Debug)]
pub struct Document {
    pub text: String,
    pub version: i32,
    pub line_index: LineIndex,
    parse: Parse,
}

impl Document {
    pub fn new(text: String, version: i32) -> Self {
        let line_index = LineIndex::new(&text);
        let parse = parse(&text);
        Document {
            text,
            version,
            line_index,
            parse,
        }
    }

    /// The current parse tree. Always present — the parser recovers rather
    /// than failing, so even a half-typed file has one.
    pub fn parse(&self) -> &Parse {
        &self.parse
    }

    /// Apply the changes from one `didChange` notification, in order.
    ///
    /// Each change's range refers to the document as of the previous change,
    /// so derived state is rebuilt between them rather than at the end.
    pub fn apply(
        &mut self,
        version: i32,
        changes: &[TextDocumentContentChangeEvent],
        enc: PositionEncoding,
    ) {
        for change in changes {
            match change.range {
                Some(range) => {
                    let start = self.line_index.offset(&self.text, range.start, enc) as usize;
                    let end = self.line_index.offset(&self.text, range.end, enc) as usize;
                    // A client that sends an inverted range is confused, but
                    // panicking on it would take the server down with it.
                    let (start, end) = if start <= end {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    self.text.replace_range(start..end, &change.text);
                }
                // No range means a full-document replacement.
                None => self.text = change.text.clone(),
            }
            self.line_index = LineIndex::new(&self.text);
        }
        self.version = version;
        self.parse = parse(&self.text);
    }
}

/// Every document the editor has told us about.
#[derive(Debug, Default)]
pub struct DocumentStore {
    docs: HashMap<Url, Document>,
}

impl DocumentStore {
    pub fn open(&mut self, uri: Url, text: String, version: i32) {
        self.docs.insert(uri, Document::new(text, version));
    }

    pub fn close(&mut self, uri: &Url) {
        self.docs.remove(uri);
    }

    pub fn get(&self, uri: &Url) -> Option<&Document> {
        self.docs.get(uri)
    }

    pub fn get_mut(&mut self, uri: &Url) -> Option<&mut Document> {
        self.docs.get_mut(uri)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Url, &Document)> {
        self.docs.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

/// The filesystem path behind a document URI, if it has one.
///
/// The index is keyed by path, so a URI that is not a file (an untitled
/// buffer, say) gets a stable synthetic path rather than being dropped —
/// otherwise its symbols would be invisible to the rest of the server.
pub fn uri_to_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|()| PathBuf::from(uri.as_str()))
}

/// The inverse of [`uri_to_path`].
///
/// A buffer that has never been saved has no path at all — the editor gives it
/// a URI like `untitled:Untitled-1` — so `uri_to_path` keeps the URI itself as
/// the key. Turning one of those back is parsing rather than path conversion,
/// which is why this cannot be `Url::from_file_path` alone. An unsaved buffer
/// is where a class is most likely to be *being written*, so dropping it here
/// would lose exactly the definitions the user is working on.
pub fn path_to_uri(path: &Path) -> Option<Url> {
    Url::from_file_path(path)
        .ok()
        .or_else(|| Url::parse(&path.to_string_lossy()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    fn change(range: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn incremental_insert() {
        let mut doc = Document::new("Foo { }".into(), 1);
        doc.apply(
            2,
            &[change(
                Some(Range::new(Position::new(0, 6), Position::new(0, 6))),
                "bar { ^1 } ",
            )],
            PositionEncoding::Utf16,
        );
        assert_eq!(doc.text, "Foo { bar { ^1 } }");
        assert_eq!(doc.version, 2);
        assert!(doc.parse().is_ok());
    }

    #[test]
    fn incremental_delete() {
        let mut doc = Document::new("Foo { bar { } }".into(), 1);
        doc.apply(
            2,
            &[change(
                Some(Range::new(Position::new(0, 6), Position::new(0, 14))),
                "",
            )],
            PositionEncoding::Utf16,
        );
        assert_eq!(doc.text, "Foo { }");
    }

    #[test]
    fn changes_apply_in_sequence() {
        // The second change's range refers to the text after the first, which
        // is the property that breaks if the line index is rebuilt too late.
        let mut doc = Document::new("a\nb\n".into(), 1);
        doc.apply(
            2,
            &[
                change(
                    Some(Range::new(Position::new(0, 0), Position::new(0, 1))),
                    "xx\ny",
                ),
                change(
                    Some(Range::new(Position::new(2, 0), Position::new(2, 1))),
                    "B",
                ),
            ],
            PositionEncoding::Utf16,
        );
        assert_eq!(doc.text, "xx\ny\nB\n");
    }

    #[test]
    fn full_replacement() {
        let mut doc = Document::new("old".into(), 1);
        doc.apply(7, &[change(None, "new text")], PositionEncoding::Utf16);
        assert_eq!(doc.text, "new text");
        assert_eq!(doc.version, 7);
    }

    #[test]
    fn edit_after_non_ascii_lands_correctly() {
        // If UTF-16 conversion were skipped, this edit would land one byte
        // early and split the emoji.
        let mut doc = Document::new("// 🎛\nFoo { }".into(), 1);
        doc.apply(
            2,
            &[change(
                Some(Range::new(Position::new(1, 0), Position::new(1, 3))),
                "Bar",
            )],
            PositionEncoding::Utf16,
        );
        assert_eq!(doc.text, "// 🎛\nBar { }");
    }

    #[test]
    fn inverted_range_does_not_panic() {
        let mut doc = Document::new("abcdef".into(), 1);
        doc.apply(
            2,
            &[change(
                Some(Range::new(Position::new(0, 4), Position::new(0, 1))),
                "-",
            )],
            PositionEncoding::Utf16,
        );
        assert_eq!(doc.text, "a-ef");
    }
}
