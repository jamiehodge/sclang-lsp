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

mod kind;
mod lexer;

pub use kind::SyntaxKind;
pub use lexer::{tokenize, Lexer, Token};
