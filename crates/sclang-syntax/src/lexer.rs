//! A lossless, error-tolerant lexer for SuperCollider.
//!
//! This is a port of the state machine in `lang/LangSource/PyrLexer.cpp`, with
//! three deliberate differences that the original has no reason to provide but
//! a language server cannot work without:
//!
//! 1. **Lossless.** Whitespace and comments are emitted as tokens rather than
//!    skipped, so the token stream covers every byte of the input exactly once
//!    and can reproduce the source verbatim.
//! 2. **Error tolerant.** Nothing aborts. Unrecognised input becomes an
//!    [`SyntaxKind::Error`] token and lexing continues, because the buffer
//!    being typed in an editor is usually not yet valid.
//! 3. **Ranges.** Every token carries a start and end byte offset. sclang's
//!    parse nodes carry only a start line and column.

use crate::kind::SyntaxKind;

/// The characters sclang treats as binary-operator constituents
/// (`PyrLexer.cpp:102`: `binopchars = "!@%&*-+=|<>?/"`).
const BINOP_CHARS: &str = "!@%&*-+=|<>?/";

fn is_binop_char(c: char) -> bool {
    BINOP_CHARS.contains(c)
}

/// sclang's identifier characters.
///
/// Non-ASCII counts, which this used to deny on the grounds that `PyrLexer.cpp`
/// is ASCII-only. sclang disagrees, and says so plainly: `±` on its own
/// compiles, `var ±x = 1;` compiles, and `1 ± 2` does not — which is the
/// behaviour of an identifier character and not of an operator. A byte above
/// ASCII lands in the identifier class of its character table.
///
/// So a name in any language lexes as a name here, rather than as an error
/// token that derails everything after it.
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii()
}

/// sclang's whitespace set (`PyrLexer.cpp` start state).
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// A lexed token: a kind plus its half-open byte range in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub start: u32,
    pub end: u32,
}

impl Token {
    /// The source text this token covers.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start as usize..self.end as usize]
    }

    /// Byte length of the token.
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Lex `input` into a complete token stream.
///
/// The result is lossless: the token ranges tile the input with no gaps and no
/// overlaps, so concatenating every token's text reproduces `input` exactly.
/// No [`SyntaxKind::Eof`] token is appended; use [`Lexer`] directly if you want
/// one.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(input);
    let mut out = Vec::new();
    while let Some(token) = lexer.next_token() {
        out.push(token);
    }
    out
}

/// A streaming lexer over a source string.
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Lexer { input, pos: 0 }
    }

    // ---- cursor primitives -------------------------------------------

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.input[self.pos..].chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Consume while `pred` holds. Returns whether anything was consumed.
    fn eat_while(&mut self, mut pred: impl FnMut(char) -> bool) -> bool {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if !pred(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        self.pos != start
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Produce the next token, or `None` at end of input.
    pub fn next_token(&mut self) -> Option<Token> {
        if self.at_end() {
            return None;
        }
        let start = self.pos;
        let kind = self.scan();
        Some(Token {
            kind,
            start: start as u32,
            end: self.pos as u32,
        })
    }

    // ---- the state machine -------------------------------------------

    fn scan(&mut self) -> SyntaxKind {
        let c = match self.bump() {
            Some(c) => c,
            None => return SyntaxKind::Eof,
        };

        match c {
            _ if is_space(c) => {
                self.eat_while(is_space);
                SyntaxKind::Whitespace
            }

            // `/` is a binop char, so comments must be checked first.
            '/' => match self.peek() {
                Some('/') => self.line_comment(),
                Some('*') => self.block_comment(),
                _ => self.binop(),
            },

            _ if is_ident_start(c) => self.ident(c),
            _ if c.is_ascii_digit() => self.number(),

            '"' => self.string(),
            '\'' => self.quoted_symbol(),
            '\\' => self.backslash_symbol(),
            '$' => self.character(),

            // `#{` opens a closed function; a bare `#` prefixes literal arrays
            // and destructuring assignment.
            '#' => {
                if self.peek() == Some('{') {
                    self.bump();
                    SyntaxKind::BeginClosedFunc
                } else {
                    SyntaxKind::Hash
                }
            }

            // `.` / `..` / `...`
            '.' => {
                if self.peek() == Some('.') {
                    self.bump();
                    if self.peek() == Some('.') {
                        self.bump();
                        SyntaxKind::Ellipsis
                    } else {
                        SyntaxKind::DotDot
                    }
                } else {
                    SyntaxKind::Dot
                }
            }

            '(' => SyntaxKind::LParen,
            ')' => SyntaxKind::RParen,
            '{' => SyntaxKind::LBrace,
            '}' => SyntaxKind::RBrace,
            '[' => SyntaxKind::LBracket,
            ']' => SyntaxKind::RBracket,
            ';' => SyntaxKind::Semicolon,
            ',' => SyntaxKind::Comma,
            ':' => SyntaxKind::Colon,
            '^' => SyntaxKind::Caret,
            '`' => SyntaxKind::Backtick,
            '~' => SyntaxKind::Tilde,

            _ if is_binop_char(c) => self.binop(),

            // Anything else: one error token, then carry on.
            _ => SyntaxKind::Error,
        }
    }

    /// `// ...` to end of line. The newline belongs to the following
    /// whitespace token, matching how editors expect comments to be ranged.
    ///
    /// Terminates on `\r` as well as `\n` (`PyrLexer.cpp` comment1:
    /// `while (c != '\n' && c != '\r' && c != 0)`). Classic Mac line endings
    /// still turn up in older quarks, and stopping only at `\n` makes a single
    /// comment swallow the whole file.
    fn line_comment(&mut self) -> SyntaxKind {
        self.bump(); // second '/'
        self.eat_while(|c| c != '\n' && c != '\r');
        SyntaxKind::LineComment
    }

    /// `/* ... */`, which **nests** in SuperCollider (`PyrLexer.cpp` comment2
    /// tracks `clevel`). An unterminated comment runs to end of input and is
    /// reported as an error rather than swallowing the file silently.
    fn block_comment(&mut self) -> SyntaxKind {
        self.bump(); // '*'
        let mut level = 1usize;
        let mut prev = '\0';
        while let Some(c) = self.bump() {
            if c == '/' && prev == '*' {
                level -= 1;
                if level == 0 {
                    return SyntaxKind::BlockComment;
                }
                prev = '\0'; // consume both characters
            } else if c == '*' && prev == '/' {
                level += 1;
                prev = '\0';
            } else {
                prev = c;
            }
        }
        SyntaxKind::Error // unterminated
    }

    /// Identifiers, keywords, class names, primitive names, and `foo:`.
    fn ident(&mut self, first: char) -> SyntaxKind {
        // The first character was already consumed, and it is not necessarily
        // one byte wide: an identifier may start with any non-ASCII character.
        let start = self.pos - first.len_utf8();
        self.eat_while(is_ident_continue);

        // An identifier immediately followed by `:` is a single keyword-binop
        // token, not an identifier and a colon.
        if self.peek() == Some(':') {
            self.bump();
            return SyntaxKind::KeywordBinop;
        }

        let text = &self.input[start..self.pos];

        // `_` alone is the partial-application argument; `_Anything` is a
        // primitive name (processident in PyrLexer.cpp).
        if text.starts_with('_') {
            return if text.len() == 1 {
                SyntaxKind::CurryArg
            } else {
                SyntaxKind::PrimitiveName
            };
        }

        match text {
            "var" => SyntaxKind::VarKw,
            "arg" => SyntaxKind::ArgKw,
            "classvar" => SyntaxKind::ClassvarKw,
            "const" => SyntaxKind::ConstKw,
            "while" => SyntaxKind::WhileKw,
            "true" => SyntaxKind::TrueKw,
            "false" => SyntaxKind::FalseKw,
            "nil" => SyntaxKind::NilKw,
            // `inf` is a float literal, not a keyword: old
            // `processident` returns SC_FLOAT and sc_lexer agrees.
            "inf" => SyntaxKind::Float,
            "pi" => SyntaxKind::PiKw,
            _ => {
                if text.starts_with(|c: char| c.is_ascii_uppercase()) {
                    SyntaxKind::ClassName
                } else {
                    SyntaxKind::Ident
                }
            }
        }
    }

    /// Numbers, following the `digits_1` state in `PyrLexer.cpp:292`.
    ///
    /// The subtle case is `.`: it only begins a float when a digit follows, so
    /// `1.5` is a float but `1.foo` is an integer followed by a method call.
    fn number(&mut self) -> SyntaxKind {
        self.eat_while(|c| c.is_ascii_digit());

        match self.peek() {
            // `16rF700`, `2r1010.11` — radix digits are 0-9 a-z A-Z.
            Some('r') => {
                self.bump();
                self.eat_while(|c| c.is_ascii_alphanumeric());
                if self.peek() == Some('.')
                    && self.peek_at(1).is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    self.bump();
                    self.eat_while(|c| c.is_ascii_alphanumeric());
                }
                SyntaxKind::RadixInteger
            }

            // `0x1F`
            Some('x') => {
                self.bump();
                self.eat_while(|c| c.is_ascii_hexdigit());
                SyntaxKind::HexInteger
            }

            // `4s`, `4ss`, `4s50`, `4b` — pitch accidentals (PyrLexer.cpp:315).
            Some(c @ ('b' | 's')) => {
                self.bump();
                if self.peek().is_some_and(|d| d.is_ascii_digit()) {
                    self.eat_while(|d| d.is_ascii_digit());
                } else {
                    self.eat_while(|d| d == c);
                }
                SyntaxKind::Accidental
            }

            Some('e' | 'E') => self.exponent(),

            Some('.') if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => {
                self.bump(); // '.'
                self.eat_while(|c| c.is_ascii_digit());
                if matches!(self.peek(), Some('e' | 'E')) {
                    self.exponent()
                } else {
                    SyntaxKind::Float
                }
            }

            _ => SyntaxKind::Integer,
        }
    }

    /// `e` has been seen but not consumed. An exponent with no digits is an
    /// error in sclang (`goto error1`), and stays one here.
    fn exponent(&mut self) -> SyntaxKind {
        self.bump(); // 'e' | 'E'
        if matches!(self.peek(), Some('+' | '-')) {
            self.bump();
        }
        if self.eat_while(|c| c.is_ascii_digit()) {
            SyntaxKind::Float
        } else {
            SyntaxKind::Error
        }
    }

    /// `"..."` with backslash escapes. Unterminated runs to end of input.
    ///
    /// One token per quoted segment. Adjacent string literals concatenate in
    /// SuperCollider, but that join is *not* done here: upstream's `sc_lexer`
    /// emits a `StringLine` per segment and leaves merging to its consumer
    /// (PyrLexer.cpp does it for bison, with the comment "this should move
    /// into the compiler making this unnecessary"). Keeping segments separate
    /// is what a lossless tree wants, so the parser joins them instead.
    fn string(&mut self) -> SyntaxKind {
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '"' => return SyntaxKind::String,
                _ => {}
            }
        }
        // Unterminated at end of input. sclang closes it there rather than
        // rejecting the file, and so does this: a half-typed string is the
        // ordinary state of a buffer, and turning the rest of it into an error
        // token helps nobody.
        SyntaxKind::String
    }

    /// `'foo'`, the quoted symbol form.
    fn quoted_symbol(&mut self) -> SyntaxKind {
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '\'' => return SyntaxKind::Symbol,
                _ => {}
            }
        }
        // Unterminated, as above.
        SyntaxKind::Symbol
    }

    /// `\foo`, `\123`, or a bare `\`. sclang accepts an alphanumeric run or a
    /// digit run, and a lone backslash is a valid empty symbol.
    fn backslash_symbol(&mut self) -> SyntaxKind {
        match self.peek() {
            Some(c) if is_ident_start(c) => {
                self.bump();
                self.eat_while(is_ident_continue);
            }
            Some(c) if c.is_ascii_digit() => {
                self.eat_while(|c| c.is_ascii_digit());
            }
            _ => {}
        }
        SyntaxKind::Symbol
    }

    /// `$a`, or `$\n` for an escaped character.
    fn character(&mut self) -> SyntaxKind {
        match self.peek() {
            Some('\\') => {
                self.bump();
                if self.bump().is_some() {
                    SyntaxKind::Char
                } else {
                    SyntaxKind::Error
                }
            }
            Some(_) => {
                self.bump();
                SyntaxKind::Char
            }
            None => SyntaxKind::Error,
        }
    }

    /// A maximal run of `binopchars`, then classified.
    ///
    /// sclang returns a handful of single characters as themselves rather than
    /// as `BINOP`, because the grammar needs them in other roles — `|` for
    /// argument lists, `<`/`>` for getter and setter markers, `*` for class
    /// methods, `-` for unary minus (`processbinop`, and lang11d:10).
    fn binop(&mut self) -> SyntaxKind {
        let start = self.pos - 1;
        self.eat_while(is_binop_char);
        match &self.input[start..self.pos] {
            "<-" => SyntaxKind::LeftArrow,
            "<>" => SyntaxKind::ReadWriteVar,
            "|" => SyntaxKind::Pipe,
            "<" => SyntaxKind::Lt,
            ">" => SyntaxKind::Gt,
            "-" => SyntaxKind::Minus,
            "*" => SyntaxKind::Star,
            "+" => SyntaxKind::Plus,
            "=" => SyntaxKind::Eq,
            _ => SyntaxKind::BinOp,
        }
    }
}
