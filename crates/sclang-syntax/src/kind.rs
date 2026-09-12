//! Token kinds, derived from sclang's own lexer.
//!
//! Every variant corresponds to something `PyrLexer.cpp` can return, plus the
//! trivia and error kinds that sclang throws away but a language server needs.
//! Line references are to `lang/LangSource/PyrLexer.cpp` in the SuperCollider
//! source tree.

/// A lexical token kind.
///
/// The set is closed: `SyntaxKind::Error` absorbs anything the lexer cannot
/// classify, so lexing never fails and never panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    // ---- Trivia -------------------------------------------------------
    // sclang skips these outright (PyrLexer.cpp:388, :859). We keep them so
    // the token stream is lossless and can round-trip to the exact source.
    /// Spaces, tabs, newlines, vertical tab, form feed.
    Whitespace,
    /// `// ... ` to end of line.
    LineComment,
    /// `/* ... */`, which nests in SuperCollider.
    BlockComment,

    // ---- Literals -----------------------------------------------------
    /// Decimal integer, e.g. `42`.
    Integer,
    /// Decimal float, e.g. `1.5`, `1e-8`, `1e+8`.
    Float,
    /// Radix literal, e.g. `16rF700`, `2r1010.11` (PyrLexer.cpp:608).
    RadixInteger,
    /// Hexadecimal literal, e.g. `0x1F` (PyrLexer.cpp:278).
    HexInteger,
    /// Pitch accidental, e.g. `4s`, `4ss`, `4s50`, `4b` (PyrLexer.cpp:315).
    Accidental,
    /// `"..."`, escapes included.
    String,
    /// `\foo` or `'foo'`.
    Symbol,
    /// `$a`, `$\n`.
    Char,

    // ---- Identifiers and names ----------------------------------------
    /// Lowercase-initial identifier (`NAME`).
    Ident,
    /// Uppercase-initial identifier (`CLASSNAME`).
    ClassName,
    /// `_Foo_Bar`, a primitive name (`PRIMITIVENAME`).
    PrimitiveName,
    /// A bare `_`, the partial-application argument (`CURRYARG`).
    CurryArg,
    /// `foo:` — an identifier immediately followed by a colon, lexed as one
    /// token (`KEYBINOP`, PyrLexer.cpp ident state).
    KeywordBinop,

    // ---- Keywords -----------------------------------------------------
    VarKw,
    ArgKw,
    ClassvarKw,
    ConstKw,
    WhileKw,
    TrueKw,
    FalseKw,
    NilKw,
    /// `inf`, which sclang returns as a float literal.
    InfKw,
    /// `pi`, the `PIE` token.
    PiKw,
    // Note: `lang11d` declares a `PSEUDOVAR` token and has a `pseudovar`
    // production, but no path in PyrLexer.cpp ever returns it. `this`,
    // `thisProcess` and friends lex as plain identifiers and are resolved
    // during compilation, so there is deliberately no token kind for them.

    // ---- Operators ----------------------------------------------------
    /// Any other run of `binopchars` (`!@%&*-+=|<>?/`, PyrLexer.cpp:102).
    BinOp,
    /// `<-`
    LeftArrow,
    /// `<>`, the read/write variable marker.
    ReadWriteVar,
    /// sclang returns these single characters as themselves rather than as
    /// `BINOP`, because the grammar needs them in other roles
    /// (lang11d:10). See `processbinop`.
    Pipe,
    Lt,
    Gt,
    Minus,
    Star,
    Plus,
    /// `=`, which binds separately (lang11d:9 `%right '='`).
    Eq,

    // ---- Punctuation --------------------------------------------------
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dot,
    /// `..`
    DotDot,
    /// `...`
    Ellipsis,
    Colon,
    /// `^`, the return marker.
    Caret,
    /// `#`
    Hash,
    /// `#{`, which opens a closed function (`BEGINCLOSEDFUNC`).
    BeginClosedFunc,
    /// `` ` ``, the ref marker.
    Backtick,
    /// `~`, which prefixes an environment variable.
    Tilde,

    // ---- Meta ---------------------------------------------------------
    /// Anything unrecognised, or an unterminated string/comment. Carries
    /// source text like any other token so losslessness holds.
    Error,
    /// Synthetic end-of-file marker.
    Eof,
}

impl SyntaxKind {
    /// Whitespace and comments: retained for losslessness, skipped by the
    /// parser.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }

    /// Any literal value.
    pub fn is_literal(self) -> bool {
        matches!(
            self,
            Self::Integer
                | Self::Float
                | Self::RadixInteger
                | Self::HexInteger
                | Self::Accidental
                | Self::String
                | Self::Symbol
                | Self::Char
        )
    }

    /// A reserved word. `ClassName` and `Ident` are deliberately excluded.
    pub fn is_keyword(self) -> bool {
        matches!(
            self,
            Self::VarKw
                | Self::ArgKw
                | Self::ClassvarKw
                | Self::ConstKw
                | Self::WhileKw
                | Self::TrueKw
                | Self::FalseKw
                | Self::NilKw
                | Self::InfKw
                | Self::PiKw
        )
    }

    /// Usable in infix position. Note that `Lt`/`Gt`/`Minus`/`Star`/`Plus`/
    /// `Pipe` are operators here *and* serve other grammatical roles.
    pub fn is_operator(self) -> bool {
        matches!(
            self,
            Self::BinOp
                | Self::LeftArrow
                | Self::ReadWriteVar
                | Self::Pipe
                | Self::Lt
                | Self::Gt
                | Self::Minus
                | Self::Star
                | Self::Plus
                | Self::Eq
                | Self::KeywordBinop
        )
    }
}
