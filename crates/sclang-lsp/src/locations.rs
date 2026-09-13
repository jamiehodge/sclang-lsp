//! Turning index locations into LSP locations.
//!
//! The index stores byte offsets into a file. Producing a line and column
//! means having that file's text, and the copy the editor is holding may
//! differ from the one on disk — so an open document always wins over the
//! file.

use crate::documents::{path_to_uri, Document, DocumentStore};
use crate::line_index::{LineIndex, PositionEncoding};
use lsp_types::{Location, Url};
use std::collections::HashMap;
use std::path::Path;

pub struct Resolver<'a> {
    docs: &'a DocumentStore,
    enc: PositionEncoding,
}

impl<'a> Resolver<'a> {
    pub fn new(docs: &'a DocumentStore, enc: PositionEncoding) -> Self {
        Resolver { docs, enc }
    }

    pub fn encoding(&self) -> PositionEncoding {
        self.enc
    }

    /// A location inside a document already in hand.
    ///
    /// Locals never leave the buffer they are written in, so there is nothing
    /// to look up and no file to read.
    pub fn in_document(&self, uri: &Url, doc: &Document, range: std::ops::Range<u32>) -> Location {
        Location {
            uri: uri.clone(),
            range: doc.line_index.range(&doc.text, range, self.enc),
        }
    }

    /// Convert one index location, reading the target file if it is not open.
    pub fn resolve(&self, loc: &sclang_index::Location) -> Option<Location> {
        self.at(&loc.file, loc.name_range.start..loc.name_range.end)
    }

    /// A byte range in a file, open or not.
    pub fn at(&self, file: &Path, range: std::ops::Range<u32>) -> Option<Location> {
        let uri = path_to_uri(file)?;

        if let Some(doc) = self.docs.get(&uri) {
            return Some(Location {
                uri,
                range: doc.line_index.range(&doc.text, range, self.enc),
            });
        }

        let text = std::fs::read_to_string(file).ok()?;
        let index = LineIndex::new(&text);
        Some(Location {
            uri,
            range: index.range(&text, range, self.enc),
        })
    }

    /// Many ranges at once, reading each file no more than once.
    ///
    /// References to something like `postln` run to thousands of occurrences
    /// spread over hundreds of files. Resolving them one at a time would read
    /// and re-index the same file for every hit in it.
    pub fn at_many<'r>(
        &self,
        items: impl IntoIterator<Item = (&'r Path, std::ops::Range<u32>)>,
    ) -> Vec<Location> {
        let mut cached: HashMap<&Path, Option<(String, LineIndex)>> = HashMap::new();
        let mut out = Vec::new();

        for (file, range) in items {
            let Some(uri) = path_to_uri(file) else {
                continue;
            };

            // An open buffer overrides the file, and its line index is already
            // built and maintained.
            if let Some(doc) = self.docs.get(&uri) {
                out.push(Location {
                    uri,
                    range: doc.line_index.range(&doc.text, range, self.enc),
                });
                continue;
            }

            let entry = cached.entry(file).or_insert_with(|| {
                let text = std::fs::read_to_string(file).ok()?;
                let index = LineIndex::new(&text);
                Some((text, index))
            });
            if let Some((text, index)) = entry {
                out.push(Location {
                    uri,
                    range: index.range(text, range, self.enc),
                });
            }
        }

        out
    }
}
