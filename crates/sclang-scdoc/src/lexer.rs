//! The SCDoc lexer, ported from `SCDoc/SCDoc.l` in the SuperCollider source
//! tree, as of 3.14.1 — the release that added `subsubsection::` and `math::`.
//!
//! flex picks, at each position, the longest match among the rules active in
//! the current start condition, and the earliest rule on a tie. That is
//! reproduced literally rather than reworked into a hand-written scanner: the
//! rules are listed below in the order `SCDoc.l` gives them, each with the
//! start conditions it is active in, and [`lex`] tries every one. A help file
//! is small, and doing exactly what flex does is worth more here than speed.
//!
//! The lexer never consults the parser — every start condition change is made
//! by a lexer action — so the whole file is tokenized up front.
//!
//! Like flex, this works on bytes. `.` matches one byte, so a multi-byte
//! character arrives as several `Text` tokens; every consumer concatenates
//! adjacent text, so they are always rejoined.

use crate::Mode;

/// A token kind, named after the `%token` declarations in `SCDoc.y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    // Header tags.
    Class,
    Title,
    Summary,
    Related,
    Categories,
    Redirect,
    // Single-line body tags.
    ClassTree,
    CopyMethod,
    Keyword,
    Private,
    // Structural tags.
    Section,
    Subsection,
    Subsubsection,
    Method,
    Argument,
    Description,
    ClassMethods,
    InstanceMethods,
    Examples,
    Returns,
    Discussion,
    // Nestable range tags.
    List,
    Tree,
    NumberedList,
    DefinitionList,
    Table,
    Footnote,
    Note,
    Warning,
    // Modal range tags.
    Code,
    Link,
    Anchor,
    Soft,
    Image,
    Teletype,
    Math,
    Strong,
    Emphasis,
    CodeBlock,
    TeletypeBlock,
    MathBlock,
    // Symbols.
    TagSym,
    Bars,
    Hashes,
    // Text.
    Text,
    Url,
    Comma,
    MethodName,
    MethodArgs,
    Newline,
    EmptyLines,
    BadMethodName,
    End,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub kind: Tok,
    /// `yylval.str`, for the kinds that set it.
    pub text: Vec<u8>,
    /// Byte offset of the token's first byte.
    pub start: usize,
}

/// flex start conditions. All are exclusive (`%x`), so a rule with no
/// condition prefix is active in `INITIAL` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Initial,
    Verbatim,
    Verbatim2,
    Metadata,
    Eat,
    Eat2,
    Eat3,
    Method,
}

impl State {
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

const I: u8 = State::Initial.bit();
const V: u8 = State::Verbatim.bit();
const V2: u8 = State::Verbatim2.bit();
const M: u8 = State::Metadata.bit();
const E: u8 = State::Eat.bit();
const E2: u8 = State::Eat2.bit();
const E3: u8 = State::Eat3.bit();
const ME: u8 = State::Method.bit();
const ALL: u8 = u8::MAX;

/// What a rule's pattern is.
#[derive(Clone, Copy)]
enum Pat {
    /// `[ \t]*name::[ \t]*`, case-insensitive.
    Tag(&'static str),
    /// `[ \t]*name::[ \t\n\r]*`, case-insensitive.
    TagNl(&'static str),
    /// `name::[ \t]*`, case-insensitive, no leading blanks.
    Inline(&'static str),
    /// `name::[ \t\n\r]*`, case-insensitive, no leading blanks.
    InlineNl(&'static str),
    /// `[ \t]*name::[ \t]*\n+`, case-insensitive.
    Block(&'static str),
    /// `[ \t\n\r]*::`
    TagSym,
    /// `\n[ \t\n\r]*::`
    NlTagSym,
    /// `\n[ \t]*\\::`
    NlEscapedTagSym,
    /// `[ \t]*` + two copies of the byte + `[ \t\n\r]*`
    Pair(u8),
    /// One literal byte sequence.
    Lit(&'static [u8]),
    /// `\n+`
    Newlines,
    /// `\n([ \t\r]*\n)+`
    EmptyLines,
    /// `[ \t]*,[ \t]*`
    Comma,
    /// `[ \t]+`
    Blanks,
    /// `[a-zA-Z]+:\/\/[^ \t\n\r:,]+`
    Url,
    /// `[a-z][a-zA-Z0-9_]*|[-<>@|&%*+/!?=]+`
    MethodName,
    /// `\([^()]+\)`
    MethodArgs,
    /// `[ \r\t]+`
    MethodBlanks,
    /// `[a-zA-Z]+`
    Alpha,
    /// `[0-9]+`
    Digits,
    /// `[.!?(){}\[\]'"0-9]+`
    Punct,
    /// `[^:\\\t\n\r ]+`
    VerbatimRun,
    /// `[^:\\\n\r,]+`
    MetadataRun,
    /// `[^:\\\n\r]`
    OneNotSpecial,
    /// `.`
    Any,
    /// `.|\n`
    AnyNl,
}

/// What a rule does when it wins.
#[derive(Clone, Copy)]
enum Act {
    /// Return the token, with `yytext` as its text.
    Ret(Tok),
    /// Return the token with a fixed text.
    RetText(&'static [u8]),
    /// Return the token and change state.
    RetTo(Tok, State),
    /// Remember the current state, enter `method`, return `METHOD`.
    Method,
    /// Return `NEWLINE` and go back to the state `method::` was seen in.
    MethodEnd,
    /// Change state, return nothing.
    To(State),
    /// Consume, return nothing.
    Skip,
}

struct Rule {
    states: u8,
    pat: Pat,
    act: Act,
}

const fn r(states: u8, pat: Pat, act: Act) -> Rule {
    Rule { states, pat, act }
}

use Act::*;
use Pat::*;
use State::{Eat, Eat2, Eat3, Initial, Metadata, Verbatim, Verbatim2};

/// The rules of `SCDoc.l`, in its order. Order matters: it breaks ties
/// between matches of equal length.
#[rustfmt::skip]
static RULES: &[Rule] = &[
    r(I, Tag("class"), Ret(Tok::Class)),
    r(I, Tag("title"), Ret(Tok::Title)),
    r(I, Tag("summary"), Ret(Tok::Summary)),
    r(I, Tag("related"), Ret(Tok::Related)),
    r(I, Tag("categories"), Ret(Tok::Categories)),
    r(I, Tag("redirect"), Ret(Tok::Redirect)),

    r(I, Tag("classtree"), Ret(Tok::ClassTree)),
    r(I, Tag("keyword"), Ret(Tok::Keyword)),

    r(I, Tag("private"), Ret(Tok::Private)),
    r(I, Tag("section"), Ret(Tok::Section)),
    r(I, Tag("subsection"), Ret(Tok::Subsection)),
    r(I, Tag("subsubsection"), Ret(Tok::Subsubsection)),
    r(I, Tag("copymethod"), Ret(Tok::CopyMethod)),
    r(I, Tag("method"), Method),
    r(I, Tag("argument"), Ret(Tok::Argument)),

    r(I, TagNl("description"), Ret(Tok::Description)),
    r(I, TagNl("classmethods"), Ret(Tok::ClassMethods)),
    r(I, TagNl("instancemethods"), Ret(Tok::InstanceMethods)),
    r(I, TagNl("examples"), Ret(Tok::Examples)),

    r(I, TagNl("returns"), Ret(Tok::Returns)),
    r(I, TagNl("discussion"), Ret(Tok::Discussion)),

    r(I, TagNl("list"), Ret(Tok::List)),
    r(I, TagNl("tree"), Ret(Tok::Tree)),
    r(I, TagNl("numberedlist"), Ret(Tok::NumberedList)),
    r(I, TagNl("definitionlist"), Ret(Tok::DefinitionList)),
    r(I, TagNl("table"), Ret(Tok::Table)),
    r(I, TagNl("footnote"), Ret(Tok::Footnote)),
    r(I, TagNl("warning"), Ret(Tok::Warning)),
    r(I, TagNl("note"), Ret(Tok::Note)),

    r(I, Inline("link"), RetTo(Tok::Link, Verbatim)),
    r(I, Inline("anchor"), RetTo(Tok::Anchor, Verbatim)),
    r(I, Inline("image"), RetTo(Tok::Image, Verbatim)),
    r(I, InlineNl("soft"), RetTo(Tok::Soft, Verbatim)),
    r(I, InlineNl("strong"), RetTo(Tok::Strong, Verbatim)),
    r(I, InlineNl("emphasis"), RetTo(Tok::Emphasis, Verbatim)),
    r(I, Inline("code"), RetTo(Tok::Code, Verbatim)),
    r(I, Inline("teletype"), RetTo(Tok::Teletype, Verbatim)),
    r(I, Inline("math"), RetTo(Tok::Math, Verbatim)),

    r(I, Block("code"), RetTo(Tok::CodeBlock, Verbatim2)),
    r(I, Block("teletype"), RetTo(Tok::TeletypeBlock, Verbatim2)),
    r(I, Block("math"), RetTo(Tok::MathBlock, Verbatim2)),

    r(I | V, TagSym, RetTo(Tok::TagSym, Initial)),
    r(V2, NlTagSym, RetTo(Tok::TagSym, Initial)),
    r(V2, NlEscapedTagSym, RetText(b"\n::")),
    r(I, Pair(b'|'), Ret(Tok::Bars)),
    r(I, Pair(b'#'), Ret(Tok::Hashes)),

    r(V2, Lit(b"\n"), RetText(b"\n")),
    r(V, Newlines, RetText(b" ")),
    r(I, Lit(b"\n"), Ret(Tok::Newline)),
    r(I, EmptyLines, Ret(Tok::EmptyLines)),

    r(I | ME, Comma, Ret(Tok::Comma)),

    r(I, Lit(b"\\||"), RetText(b"||")),
    r(I, Lit(b"\\##"), RetText(b"##")),
    r(I | V, Lit(b"\\::"), RetText(b"::")),
    r(V | V2, Lit(b"\t"), RetText(b"    ")),
    r(I, Blanks, RetText(b" ")),

    r(ALL, Lit(b"\r"), Skip),

    r(I, Url, Ret(Tok::Url)),
    r(ME, MethodName, Ret(Tok::MethodName)),
    r(ME, MethodArgs, Ret(Tok::MethodArgs)),
    r(ME, MethodBlanks, Skip),
    r(ME, Lit(b"\n"), MethodEnd),
    r(ME, Any, Ret(Tok::BadMethodName)),

    r(I, Alpha, Ret(Tok::Text)),
    r(I | V | V2, Punct, Ret(Tok::Text)),
    r(V | V2, VerbatimRun, Ret(Tok::Text)),
    r(I, OneNotSpecial, Ret(Tok::Text)),
    r(I | V | V2, Any, Ret(Tok::Text)),

    r(M | E, Tag("class"), RetTo(Tok::Class, Metadata)),
    r(M | E, Tag("title"), RetTo(Tok::Title, Metadata)),
    r(M | E, Tag("summary"), RetTo(Tok::Summary, Metadata)),
    r(M | E, Tag("related"), RetTo(Tok::Related, Metadata)),
    r(M | E, Tag("categories"), RetTo(Tok::Categories, Metadata)),
    r(M | E, Tag("redirect"), RetTo(Tok::Redirect, Metadata)),
    r(M | E, Tag("classtree"), RetTo(Tok::ClassTree, Metadata)),
    r(M | E, Tag("keyword"), RetTo(Tok::Keyword, Metadata)),
    r(M | E, Tag("private"), RetTo(Tok::Private, Metadata)),
    r(M | E, Tag("section"), RetTo(Tok::Section, Metadata)),
    r(M | E, Tag("subsection"), RetTo(Tok::Subsection, Metadata)),
    r(M | E, Tag("subsubsection"), RetTo(Tok::Subsubsection, Metadata)),
    r(M | E, Tag("copymethod"), RetTo(Tok::CopyMethod, Metadata)),
    r(M | E, Tag("method"), Method),
    r(M | E, TagNl("description"), RetTo(Tok::Description, Eat)),
    r(M | E, TagNl("classmethods"), RetTo(Tok::ClassMethods, Eat)),
    r(M | E, TagNl("instancemethods"), RetTo(Tok::InstanceMethods, Eat)),
    r(M | E, TagNl("examples"), RetTo(Tok::Examples, Eat)),
    r(M, Lit(b"\n"), RetTo(Tok::Newline, Eat)),
    r(M, Comma, Ret(Tok::Comma)),
    r(M, Alpha, Ret(Tok::Text)),
    r(M, Digits, Ret(Tok::Text)),
    r(M, MetadataRun, Ret(Tok::Text)),
    r(M, Any, Ret(Tok::Text)),
    r(M, Lit(b"\\::"), RetText(b"::")),
    r(E, Inline("link"), To(Eat2)),
    r(E, Inline("anchor"), To(Eat2)),
    r(E, Inline("image"), To(Eat2)),
    r(E, InlineNl("soft"), To(Eat2)),
    r(E, InlineNl("strong"), To(Eat2)),
    r(E, InlineNl("emphasis"), To(Eat2)),
    r(E, Inline("code"), To(Eat2)),
    r(E, Inline("teletype"), To(Eat2)),
    r(E, Inline("math"), To(Eat2)),
    r(E, Block("code"), To(Eat3)),
    r(E, Block("teletype"), To(Eat3)),
    r(E, Block("math"), To(Eat3)),
    r(E2, TagSym, To(Eat)),
    r(E3, NlTagSym, To(Eat)),
    r(E | E2 | E3, AnyNl, Skip),
];

/// Tokenize a help file. The first token is the start token for `mode`, as
/// `scdoc_start_token` arranges, and the last is always [`Tok::End`].
pub(crate) fn lex(src: &[u8], mode: Mode) -> Vec<Token> {
    let mut state = match mode {
        // `if(t==START_METADATA) BEGIN(eat);`
        Mode::Metadata => State::Eat,
        Mode::Full | Mode::Partial => State::Initial,
    };
    let mut method_caller = State::Initial;
    let mut out = Vec::new();
    let mut pos = 0;

    while pos < src.len() {
        let mut best: Option<(usize, &Rule)> = None;
        for rule in RULES {
            if rule.states & state.bit() == 0 {
                continue;
            }
            if let Some(len) = matches(rule.pat, &src[pos..]) {
                // Strictly longer, so the earlier rule keeps a tie.
                if len > 0 && best.is_none_or(|(l, _)| len > l) {
                    best = Some((len, rule));
                }
            }
        }

        // Every state has a rule matching any single byte except `\n`, and
        // every state but `metadata` one matching `\n`. flex's default rule
        // echoes what nothing matched; there is no output to echo to, so
        // skipping the byte is the closest thing.
        let Some((len, rule)) = best else {
            pos += 1;
            continue;
        };
        let lexeme = &src[pos..pos + len];
        let token = |kind, text: &[u8]| Token {
            kind,
            text: text.to_vec(),
            start: pos,
        };
        match rule.act {
            Ret(kind) => out.push(token(kind, lexeme)),
            RetText(text) => out.push(token(Tok::Text, text)),
            RetTo(kind, to) => {
                state = to;
                out.push(token(kind, lexeme));
            }
            Method => {
                method_caller = state;
                state = State::Method;
                out.push(token(Tok::Method, lexeme));
            }
            MethodEnd => {
                state = method_caller;
                out.push(token(Tok::Newline, lexeme));
            }
            To(to) => state = to,
            Skip => {}
        }
        pos += len;
    }

    out.push(Token {
        kind: Tok::End,
        text: Vec::new(),
        start: src.len(),
    });
    out
}

/// The length of `pat`'s longest match at the start of `s`.
fn matches(pat: Pat, s: &[u8]) -> Option<usize> {
    let blanks = |s: &[u8]| span(s, |b| b == b' ' || b == b'\t');
    let space = |s: &[u8]| span(s, |b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));

    match pat {
        Tag(name) => {
            let n = blanks(s);
            let n = n + tag_name(&s[n..], name)?;
            Some(n + blanks(&s[n..]))
        }
        TagNl(name) => {
            let n = blanks(s);
            let n = n + tag_name(&s[n..], name)?;
            Some(n + space(&s[n..]))
        }
        Inline(name) => {
            let n = tag_name(s, name)?;
            Some(n + blanks(&s[n..]))
        }
        InlineNl(name) => {
            let n = tag_name(s, name)?;
            Some(n + space(&s[n..]))
        }
        Block(name) => {
            let n = blanks(s);
            let n = n + tag_name(&s[n..], name)?;
            let n = n + blanks(&s[n..]);
            let newlines = span(&s[n..], |b| b == b'\n');
            (newlines > 0).then_some(n + newlines)
        }
        TagSym => {
            let n = space(s);
            s[n..].starts_with(b"::").then_some(n + 2)
        }
        NlTagSym => {
            let rest = s.strip_prefix(b"\n")?;
            let n = space(rest);
            rest[n..].starts_with(b"::").then_some(1 + n + 2)
        }
        NlEscapedTagSym => {
            let rest = s.strip_prefix(b"\n")?;
            let n = blanks(rest);
            rest[n..].starts_with(b"\\::").then_some(1 + n + 3)
        }
        Pair(c) => {
            let n = blanks(s);
            (s.get(n) == Some(&c) && s.get(n + 1) == Some(&c)).then(|| n + 2 + space(&s[n + 2..]))
        }
        Lit(lit) => s.starts_with(lit).then_some(lit.len()),
        Newlines => nonempty(span(s, |b| b == b'\n')),
        EmptyLines => {
            // `\n([ \t\r]*\n)+`: the longest prefix ending in a newline that
            // has at least two of them and only blanks between.
            let rest = s.strip_prefix(b"\n")?;
            let mut end = None;
            let mut i = 0;
            while let Some(&b) = rest.get(i) {
                match b {
                    b'\n' => end = Some(i + 1),
                    b' ' | b'\t' | b'\r' => {}
                    _ => break,
                }
                i += 1;
            }
            end.map(|e| 1 + e)
        }
        Comma => {
            let n = blanks(s);
            (s.get(n) == Some(&b',')).then(|| n + 1 + blanks(&s[n + 1..]))
        }
        Blanks => nonempty(blanks(s)),
        Url => {
            let n = span(s, |b| b.is_ascii_alphabetic());
            if n == 0 || !s[n..].starts_with(b"://") {
                return None;
            }
            let m = span(&s[n + 3..], |b| {
                !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b':' | b',')
            });
            nonempty(m).map(|m| n + 3 + m)
        }
        MethodName => {
            let word = match s.first() {
                Some(b) if b.is_ascii_lowercase() => {
                    1 + span(&s[1..], |b| b.is_ascii_alphanumeric() || b == b'_')
                }
                _ => 0,
            };
            let op = span(s, |b| b"-<>@|&%*+/!?=".contains(&b));
            nonempty(word.max(op))
        }
        MethodArgs => {
            let rest = s.strip_prefix(b"(")?;
            let n = span(rest, |b| b != b'(' && b != b')');
            (n > 0 && rest.get(n) == Some(&b')')).then_some(n + 2)
        }
        MethodBlanks => nonempty(span(s, |b| matches!(b, b' ' | b'\r' | b'\t'))),
        Alpha => nonempty(span(s, |b| b.is_ascii_alphabetic())),
        Digits => nonempty(span(s, |b| b.is_ascii_digit())),
        Punct => nonempty(span(s, |b| {
            b.is_ascii_digit() || b".!?(){}[]'\"".contains(&b)
        })),
        VerbatimRun => nonempty(span(s, |b| {
            !matches!(b, b':' | b'\\' | b'\t' | b'\n' | b'\r' | b' ')
        })),
        MetadataRun => nonempty(span(s, |b| {
            !matches!(b, b':' | b'\\' | b'\n' | b'\r' | b',')
        })),
        OneNotSpecial => match s.first() {
            Some(b':' | b'\\' | b'\n' | b'\r') | None => None,
            Some(_) => Some(1),
        },
        Any => match s.first() {
            Some(b'\n') | None => None,
            Some(_) => Some(1),
        },
        AnyNl => (!s.is_empty()).then_some(1),
    }
}

/// `name::`, with the name matched case-insensitively as `(?i:…)` does.
fn tag_name(s: &[u8], name: &str) -> Option<usize> {
    let n = name.len();
    let head = s.get(..n)?;
    (head.eq_ignore_ascii_case(name.as_bytes()) && s[n..].starts_with(b"::")).then_some(n + 2)
}

fn span(s: &[u8], f: impl Fn(u8) -> bool) -> usize {
    s.iter().take_while(|&&b| f(b)).count()
}

fn nonempty(n: usize) -> Option<usize> {
    (n > 0).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, mode: Mode) -> Vec<Tok> {
        lex(src.as_bytes(), mode)
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn longest_match_beats_rule_order() {
        // `[a-zA-Z]+` comes later than the tag rules but would match `class`
        // alone; `classmethods::` is longer than `class` and wins.
        assert_eq!(
            kinds("classmethods::\n", Mode::Full),
            [Tok::ClassMethods, Tok::End]
        );
        assert_eq!(
            kinds("myclass::", Mode::Full),
            [Tok::Text, Tok::TagSym, Tok::End]
        );
    }

    #[test]
    fn tags_are_case_insensitive() {
        assert_eq!(
            kinds("Description::", Mode::Full),
            [Tok::Description, Tok::End]
        );
    }

    #[test]
    fn a_code_block_closes_only_at_the_start_of_a_line() {
        let toks = lex(b"code::\na::b\n::", Mode::Full);
        let text: Vec<u8> = toks
            .iter()
            .filter(|t| t.kind == Tok::Text)
            .flat_map(|t| t.text.clone())
            .collect();
        assert_eq!(text, b"a::b");
        assert_eq!(toks.last().unwrap().kind, Tok::End);
        assert_eq!(toks[toks.len() - 2].kind, Tok::TagSym);
    }

    #[test]
    fn method_state_returns_to_its_caller() {
        assert_eq!(
            kinds("method:: ar, kr (a)\nfoo", Mode::Full),
            [
                Tok::Method,
                Tok::MethodName,
                Tok::Comma,
                Tok::MethodName,
                Tok::MethodArgs,
                Tok::Newline,
                Tok::Text,
                Tok::End
            ]
        );
    }

    #[test]
    fn carriage_returns_vanish() {
        assert_eq!(kinds("a\r\nb", Mode::Full), kinds("a\nb", Mode::Full));
    }
}
