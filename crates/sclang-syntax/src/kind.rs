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

    // =================================================================
    // Node kinds. Everything above is produced by the lexer; everything
    // below is produced by the parser. They share one enum so the tree can
    // hold both, which is the usual arrangement for a lossless CST.
    //
    // Names follow the `lang11d` productions they come from, so the grammar
    // and this list can be read side by side.
    // =================================================================
    /// The whole file (`root`).
    SourceFile,

    // ---- Top level ----------------------------------------------------
    /// `Foo : Bar { ... }` (`classdef`).
    ClassDef,
    /// `Array[slot] : ArrayedCollection { ... }` — the indexed form's
    /// `[slot]` part (`optname`).
    IndexedSlot,
    /// `: Bar` (`superclass`).
    SuperClass,
    /// `+ Foo { ... }` (`classextension`).
    ClassExtension,

    // ---- Class members ------------------------------------------------
    /// `classvar <a, b;` / `var <>x;` / `const c = 1;` (`classvardecl`).
    ClassVarDecl,
    /// One `<name = value` entry in such a declaration (`rwslotdef`).
    SlotDef,
    /// The `<`, `>` or `<>` getter/setter marker on a slot.
    RwSpec,
    /// `foo { ... }`, `*foo { ... }`, `++ { ... }` (`methoddef`).
    MethodDef,
    /// `_Prim_Name` at the head of a method body (`primitive`).
    Primitive,

    // ---- Declarations inside a function -------------------------------
    /// `arg a, b = 1;` or `|a, b = 1|` (`argdecls`).
    ArgDecls,
    /// `var a, b = 1;` (`funcvardecl`).
    VarDecls,
    /// One `name = default` entry in either of the above (`slotdef`/`vardef`).
    VarDef,
    /// `...rest` in an argument list.
    RestArg,

    // ---- Expressions --------------------------------------------------
    /// A sequence of `;`-separated expressions (`exprseq`).
    ExprSeq,
    /// `^expr` (`retval`).
    ReturnStmt,
    /// `a + b`, including the optional adverb (`expr binop2 adverb expr`).
    BinaryExpr,
    /// `.x` or `.(expr)` on a binary operator (`adverb`).
    Adverb,
    /// `-a` (`UMINUS`).
    UnaryExpr,
    /// `a = b`, in all its forms.
    AssignExpr,
    /// `#a, b = c` — destructuring assignment (`'#' mavars '=' expr`).
    MultiAssignExpr,
    /// The `#a, b ...c` target list of a destructuring assignment.
    MultiAssignTargets,
    /// `foo(a, b)` or `foo { }`.
    CallExpr,
    /// `a.foo(b)`.
    MethodCall,
    /// The parenthesised or trailing-block arguments of a call.
    ArgList,
    /// `key: value` inside an argument list (`keyarg`).
    KeywordArg,
    /// `*args` — array expansion in an argument list (`arglistv1`).
    SplatArg,
    /// `a[1..2]`, `a[1..]`, `a[..2]` (`valrangex1`).
    IndexRange,
    /// `a[i]` (`expr1 '[' arglist1 ']'`).
    IndexExpr,
    /// `a.[i]` — the `at` shorthand.
    DotIndexExpr,
    /// `{ ... }`, a function literal (`block`).
    FunctionBlock,
    /// `{: expr, x <- (0..3) }` — a list comprehension (`generator`).
    Generator,
    /// One `x <- xs`, `var a = b`, or guard of a list comprehension (`qual`).
    Qualifier,
    /// `( ... )`, a parenthesised expression or an event literal.
    ParenExpr,
    /// `(a, b .. c)` (`valrangexd`).
    ArithSeries,
    /// `[a, b, c]` or `Set[a, b]`.
    Collection,
    /// `#[a, b]` (`listlit`).
    LiteralList,
    /// `(a: 1, b: 2)`, an event literal (`dictslotlist`).
    EventLiteral,
    /// `` `expr `` (`'`' expr`).
    RefExpr,
    /// A literal token wrapped as an expression node.
    Literal,
    /// A bare identifier used as a value.
    NameRef,
    /// `~foo`.
    EnvVarRef,
    /// A class name used as a value.
    ClassRef,

    /// A node the parser could not make sense of. Its children are retained
    /// so the tree stays lossless.
    ErrorNode,
}

impl SyntaxKind {
    /// True for kinds the lexer produces, false for kinds the parser produces.
    pub fn is_token(self) -> bool {
        (self as u16) <= (Self::Eof as u16)
    }

    /// True for internal tree nodes.
    pub fn is_node(self) -> bool {
        !self.is_token()
    }

    /// Whitespace and comments: retained for losslessness, skipped by the
    /// parser.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }

    /// Any literal value.
    ///
    /// `true`, `false`, `nil` and `pi` are here as well as in
    /// [`Self::is_keyword`], and both are true of them: the lexer gives each
    /// its own kind because it recognises the word, and `lang11d`'s `literal`
    /// production lists all four. They name a value, not a construct.
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
                | Self::TrueKw
                | Self::FalseKw
                | Self::NilKw
                | Self::PiKw
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
