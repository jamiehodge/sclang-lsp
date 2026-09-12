//! Syntax layer for SuperCollider.
//!
//! The lexer here is a port of `lang/LangSource/PyrLexer.cpp` from the
//! SuperCollider source tree, and the parser that will sit on top of it is
//! derived from the same project's bison grammar (`Bison/lang11d`). Deriving
//! both from sclang's own front end is what lets this crate claim fidelity;
//! the departures from it are limited to what an editor needs, and each one is
//! marked at the site where it happens.
//!
//! ```
//! use sclang_syntax::{tokenize, SyntaxKind};
//!
//! let tokens = tokenize("Foo { bar { ^1 + 2 } }");
//! assert_eq!(tokens[0].kind, SyntaxKind::ClassName);
//! ```

mod grammar;
mod kind;
mod lexer;
mod parser;
mod tree;

pub use kind::SyntaxKind;
pub use lexer::{tokenize, Lexer, Token};
pub use tree::{Child, Parse, SyntaxError, SyntaxNode};

/// Parse SuperCollider source into a lossless concrete syntax tree.
///
/// Always returns a tree. Syntax errors are collected in [`Parse::errors`]
/// rather than aborting, because the buffer an editor hands a language server
/// is usually not yet valid.
///
/// ```
/// let parse = sclang_syntax::parse("Foo : Bar { baz { ^1 } }");
/// assert!(parse.is_ok());
/// ```
pub fn parse(source: &str) -> Parse {
    let tokens = tokenize(source);
    let mut p = parser::Parser::new(source, &tokens);
    grammar::source_file(&mut p);
    let (events, errors) = p.finish();
    parser::build_tree(source, &tokens, events, errors)
}
