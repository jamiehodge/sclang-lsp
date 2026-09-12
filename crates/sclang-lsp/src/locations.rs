//! Turning index locations into LSP locations.
//!
//! The index stores byte offsets into a file. Producing a line and column
//! means having that file's text, and the copy the editor is holding may
//! differ from the one on disk — so an open document always wins over the
//! file.

use crate::documents::DocumentStore;
use crate::line_index::{LineIndex, PositionEncoding};
use lsp_types::{Location, Url};

pub struct Resolver<'a> {
    docs: &'a DocumentStore,
    enc: PositionEncoding,
}

impl<'a> Resolver<'a> {
    pub fn new(docs: &'a DocumentStore, enc: PositionEncoding) -> Self {
        Resolver { docs, enc }
    }

    /// Convert one index location, reading the target file if it is not open.
    pub fn resolve(&self, loc: &sclang_index::Location) -> Option<Location> {
        let uri = Url::from_file_path(&loc.file).ok()?;
        let range = loc.name_range.start..loc.name_range.end;

        if let Some(doc) = self.docs.get(&uri) {
            return Some(Location {
                uri,
                range: doc.line_index.range(&doc.text, range, self.enc),
            });
        }

        let text = std::fs::read_to_string(&loc.file).ok()?;
        let index = LineIndex::new(&text);
        Some(Location {
            uri,
            range: index.range(&text, range, self.enc),
        })
    }
}
