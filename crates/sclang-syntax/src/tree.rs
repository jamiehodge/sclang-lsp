//! A lossless concrete syntax tree.
//!
//! Every byte of the source appears in exactly one token in the tree,
//! including whitespace and comments, so the tree round-trips to the original
//! text. That property is what makes formatting, refactoring, and precise
//! hover ranges possible later; it is also the easiest invariant to test.

use crate::kind::SyntaxKind;
use crate::lexer::Token;
use std::fmt;

/// A child of a node: either a nested node or a leaf token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Child {
    Node(SyntaxNode),
    Token(Token),
}

impl Child {
    pub fn kind(&self) -> SyntaxKind {
        match self {
            Child::Node(n) => n.kind,
            Child::Token(t) => t.kind,
        }
    }

    pub fn range(&self) -> (u32, u32) {
        match self {
            Child::Node(n) => (n.start, n.end),
            Child::Token(t) => (t.start, t.end),
        }
    }
}

/// An internal node: a kind, a byte range, and children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxNode {
    pub kind: SyntaxKind,
    pub start: u32,
    pub end: u32,
    pub children: Vec<Child>,
}

impl SyntaxNode {
    /// The source text this node spans.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start as usize..self.end as usize]
    }

    /// Direct child nodes, skipping tokens.
    pub fn child_nodes(&self) -> impl Iterator<Item = &SyntaxNode> {
        self.children.iter().filter_map(|c| match c {
            Child::Node(n) => Some(n),
            Child::Token(_) => None,
        })
    }

    /// Direct child tokens, skipping nodes.
    pub fn child_tokens(&self) -> impl Iterator<Item = &Token> {
        self.children.iter().filter_map(|c| match c {
            Child::Token(t) => Some(t),
            Child::Node(_) => None,
        })
    }

    /// The first direct child node of the given kind.
    pub fn child_of(&self, kind: SyntaxKind) -> Option<&SyntaxNode> {
        self.child_nodes().find(|n| n.kind == kind)
    }

    /// The first direct child token of the given kind.
    pub fn token_of(&self, kind: SyntaxKind) -> Option<&Token> {
        self.child_tokens().find(|t| t.kind == kind)
    }

    /// Every node in the subtree, this one first, in source order.
    pub fn descendants(&self) -> Vec<&SyntaxNode> {
        let mut out = Vec::new();
        self.collect_descendants(&mut out);
        out
    }

    fn collect_descendants<'a>(&'a self, out: &mut Vec<&'a SyntaxNode>) {
        out.push(self);
        for child in &self.children {
            if let Child::Node(n) = child {
                n.collect_descendants(out);
            }
        }
    }

    /// An indented s-expression, for tests and debugging. Trivia is omitted
    /// so that expected trees stay readable.
    pub fn debug_tree(&self, source: &str) -> String {
        let mut out = String::new();
        self.write_debug(source, 0, &mut out);
        out
    }

    fn write_debug(&self, source: &str, indent: usize, out: &mut String) {
        use fmt::Write;
        let _ = writeln!(out, "{:indent$}{:?}", "", self.kind, indent = indent);
        for child in &self.children {
            match child {
                Child::Node(n) => n.write_debug(source, indent + 2, out),
                Child::Token(t) if !t.kind.is_trivia() => {
                    let _ = writeln!(
                        out,
                        "{:indent$}{:?} {:?}",
                        "",
                        t.kind,
                        t.text(source),
                        indent = indent + 2
                    );
                }
                Child::Token(_) => {}
            }
        }
    }
}

/// A syntax error with the byte range it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

/// The result of parsing a file: a tree that always exists, plus whatever
/// went wrong while building it.
#[derive(Debug, Clone)]
pub struct Parse {
    pub root: SyntaxNode,
    pub errors: Vec<SyntaxError>,
}

impl Parse {
    /// Whether the source parsed without any errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}
