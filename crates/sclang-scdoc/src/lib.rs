//! SCDoc, SuperCollider's help-file format.
//!
//! A port of upstream's own parser: the flex lexer `SCDoc/SCDoc.l`, the bison
//! grammar `SCDoc/SCDoc.y`, and the `doc_node_fixup_tree` pass from
//! `SCDoc/SCDoc.cpp`. The tree is the one sclang builds when it reads a
//! `.schelp` file, node for node, which `oracle/build-scdoc.sh` and the
//! `scdoc_oracle` example check against upstream's code directly.
//!
//! ```
//! use sclang_scdoc::{parse, Id, Mode};
//!
//! let doc = parse("class:: Foo\nsummary:: A foo.\n", Mode::Full).unwrap();
//! let header = doc.child(Id::Header).unwrap();
//! assert_eq!(header.child(Id::Summary).unwrap().text.as_deref(), Some("A foo."));
//! ```

mod lexer;
pub mod markdown;
mod parser;
mod tree;

pub use tree::{Id, Node};

/// Which of upstream's three entry points to parse with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A whole help file: a header, then sections. What `SCDoc.parseFileFull`
    /// reads.
    Full,
    /// Sections only, with no header. What `SCDoc.parseFilePartial` reads, and
    /// what `.ext.schelp` files — a quark's additions to another class's
    /// page — are written in.
    Partial,
    /// The header and the method names, skipping all prose. What sclang
    /// indexes every help file with.
    Metadata,
}

/// Why a help file did not parse.
///
/// Upstream recovers from nothing: one error and the whole file is rejected,
/// which is what is reproduced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Byte offset of the token the error was found at.
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for Error {}

/// Parse a help file. Takes bytes rather than `&str` because upstream does:
/// a help file that is not valid UTF-8 still parses, and its text comes back
/// with the invalid sequences replaced.
pub fn parse(source: impl AsRef<[u8]>, mode: Mode) -> Result<Node, Error> {
    let source = source.as_ref();
    let tokens = lexer::lex(source, mode);
    let mut root = parser::parse(source, &tokens, mode)?;
    tree::fixup(&mut root);
    Ok(root.into_node())
}
