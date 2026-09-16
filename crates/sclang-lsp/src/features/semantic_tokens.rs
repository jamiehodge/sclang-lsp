//! Semantic tokens: colour decided by the parse tree rather than by regexes.
//!
//! An editor's own grammar has to guess from shape alone. `foo` is a variable
//! if it is lowercase; `Foo` is a class if it is capitalised. That is wrong
//! often enough to notice: in `x.blend(1)` the selector is not a variable, in
//! `foo(a)` the callee is a method — sclang reads it as `a.foo` — and a name
//! declared as `arg` is a parameter wherever it later appears. The tree knows
//! all three, and this is how it says so.
//!
//! **Nothing here consults the symbol index.** Everything comes from the
//! buffer's own tree, so colour is identical before and after the class
//! library scan lands. Resolving class names against the index would let a
//! known class differ from an unknown one, at the cost of every open file
//! changing colour part-way through startup — a flicker for information the
//! diagnostics already give properly.
//!
//! **Full coverage, not just the semantic part.** A client that also has a
//! grammar sets `augmentsSyntaxTokens` and layers ours on top, so emitting
//! comments, literals and operators costs nothing there; a client with no
//! SuperCollider grammar at all gets highlighting it otherwise has no source
//! for. The only tokens deliberately left out are punctuation, which no theme
//! colours anyway.
//!
//! [`Cache`] is what makes `full/delta` possible: the array last sent for each
//! open document, labelled with the result id the client was given. Rebuilding
//! the tokens costs well under a millisecond either way — what a delta saves is
//! the *transfer*, and on a class-library file that is a few hundred kilobytes
//! of JSON on every keystroke rather than a handful of integers.

use crate::documents::Document;
use crate::line_index::PositionEncoding;
use crate::scope::{class_slots, declared_in, is_scope, Local, LocalKind, PSEUDO_VARIABLES};
use lsp_types::{
    Range, SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensDelta, SemanticTokensEdit, SemanticTokensFullDeltaResult, SemanticTokensLegend,
    Url,
};
use sclang_index::SymbolIndex;
use sclang_syntax::{Child, SyntaxKind, SyntaxNode, Token};
use std::collections::HashMap;

/// The token types this server emits, in the order the legend declares them.
///
/// Only names from the protocol's predefined set. A client may accept custom
/// types, but no stock theme has a rule to colour one with, so a custom name
/// buys a token that is invisible almost everywhere.
const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::CLASS,
    SemanticTokenType::METHOD,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::MODIFIER,
    SemanticTokenType::COMMENT,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::MACRO,
];

const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::READONLY,
    SemanticTokenModifier::STATIC,
];

/// An index into [`TOKEN_TYPES`]. The discriminants are the wire values, which
/// is why the order of the two lists has to agree — `legend_matches_types`
/// holds them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    Class = 0,
    Method,
    Parameter,
    Variable,
    Property,
    EnumMember,
    Keyword,
    Modifier,
    Comment,
    String,
    Number,
    Operator,
    Macro,
}

/// Bits into [`TOKEN_MODIFIERS`].
const DECLARATION: u32 = 1 << 0;
const READONLY: u32 = 1 << 1;
const STATIC: u32 = 1 << 2;

/// What the server advertises at `initialize`. The client keeps this for the
/// life of the session and decodes every response against it.
pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Every token in the document.
///
/// Bare tokens, with no result id: labelling a response is [`Cache`]'s job,
/// because only it knows whether the array can be diffed against later.
pub fn semantic_tokens(
    doc: &Document,
    index: &SymbolIndex,
    enc: PositionEncoding,
) -> Vec<SemanticToken> {
    tokens_in(doc, index, 0, doc.text.len() as u32, enc)
}

/// The tokens of one span, for a client painting the viewport first.
pub fn semantic_tokens_range(
    doc: &Document,
    index: &SymbolIndex,
    range: Range,
    enc: PositionEncoding,
) -> Vec<SemanticToken> {
    let from = doc.line_index.offset(&doc.text, range.start, enc);
    let to = doc.line_index.offset(&doc.text, range.end, enc);
    tokens_in(doc, index, from, to, enc)
}

fn tokens_in(
    doc: &Document,
    index: &SymbolIndex,
    from: u32,
    to: u32,
    enc: PositionEncoding,
) -> Vec<SemanticToken> {
    let mut walker = Walker {
        source: &doc.text,
        index,
        from,
        to,
        scopes: Vec::new(),
        slots: Vec::new(),
        out: Vec::new(),
    };
    walker.walk(&doc.parse().root, None);
    encode(&walker.out, doc, enc)
}

/// What was last sent for each open document, and under which result id.
///
/// This is the whole of `full/delta`: a client hands back the id it was last
/// given, and gets the edits since rather than the array again. Nothing here
/// is invalidated by an edit — being able to diff *against* the last response
/// is the point — so the only lifecycle event that matters is closing a
/// document, which is [`Cache::forget`].
#[derive(Debug, Default)]
pub struct Cache {
    sent: HashMap<Url, Sent>,
    /// Result ids only have to be unique per document, but a single counter
    /// is simpler and makes them unique across the session, which is easier
    /// to follow in a protocol trace.
    issued: u64,
}

#[derive(Debug)]
struct Sent {
    result_id: String,
    tokens: Vec<SemanticToken>,
}

impl Cache {
    /// A full response, labelled so the next request can ask for a delta.
    pub fn full(&mut self, uri: &Url, tokens: Vec<SemanticToken>) -> SemanticTokens {
        let result_id = self.remember(uri, tokens.clone());
        SemanticTokens {
            result_id: Some(result_id),
            data: tokens,
        }
    }

    /// The edits since `previous_result_id`, or the whole array when that is
    /// not what this document was last sent.
    ///
    /// A stale id is not an error: the client may have been talking to a
    /// server that has since restarted, or to a document that was closed and
    /// reopened. The protocol's answer to both is a full response.
    pub fn delta(
        &mut self,
        uri: &Url,
        previous_result_id: &str,
        tokens: Vec<SemanticToken>,
    ) -> SemanticTokensFullDeltaResult {
        let against = match self.sent.get(uri) {
            Some(sent) if sent.result_id == previous_result_id => {
                Some(edits(&sent.tokens, &tokens))
            }
            _ => None,
        };

        match against {
            Some(edits) => {
                let result_id = self.remember(uri, tokens);
                SemanticTokensDelta {
                    result_id: Some(result_id),
                    edits,
                }
                .into()
            }
            None => self.full(uri, tokens).into(),
        }
    }

    /// Drop what a closed document was holding. A range response is never
    /// remembered, so there is nothing else to clear.
    pub fn forget(&mut self, uri: &Url) {
        self.sent.remove(uri);
    }

    fn remember(&mut self, uri: &Url, tokens: Vec<SemanticToken>) -> String {
        self.issued += 1;
        let result_id = self.issued.to_string();
        self.sent.insert(
            uri.clone(),
            Sent {
                result_id: result_id.clone(),
                tokens,
            },
        );
        result_id
    }
}

/// The edits that turn one token array into another.
///
/// One edit: keep the longest common prefix and suffix, replace everything
/// between. A finer diff is possible and not worth it, because the relative
/// encoding has already localised the change — every token's position is
/// relative to the token before it, so an edit on one line leaves every token
/// on later lines byte-identical, and the middle stays small for the edits
/// people actually make.
fn edits(previous: &[SemanticToken], current: &[SemanticToken]) -> Vec<SemanticTokensEdit> {
    let prefix = previous
        .iter()
        .zip(current)
        .take_while(|(before, after)| before == after)
        .count();

    if prefix == previous.len() && prefix == current.len() {
        return Vec::new();
    }

    // The suffix may not reach back into the prefix, or the same tokens would
    // be both kept and deleted.
    let available = previous.len().min(current.len()) - prefix;
    let suffix = previous
        .iter()
        .rev()
        .zip(current.iter().rev())
        .take(available)
        .take_while(|(before, after)| before == after)
        .count();

    vec![SemanticTokensEdit {
        // The protocol indexes the flat array of integers, five per token.
        start: (prefix * 5) as u32,
        delete_count: ((previous.len() - prefix - suffix) * 5) as u32,
        data: Some(current[prefix..current.len() - suffix].to_vec()),
    }]
}

/// One classified span of source, before delta encoding.
struct Span {
    start: u32,
    end: u32,
    ty: Type,
    modifiers: u32,
}

struct Walker<'a> {
    source: &'a str,
    index: &'a SymbolIndex,
    from: u32,
    to: u32,
    /// Innermost scope last. Only the declarations of the scopes actually
    /// enclosing the current node, which is what makes shadowing come out
    /// right and keeps this one pass rather than one lookup per name.
    scopes: Vec<Vec<Local>>,
    /// The slots of the class being walked, inherited ones included. Not a
    /// scope: they come from the index rather than the tree, and classes do
    /// not nest, so one set at a time is all there is.
    slots: Vec<(String, LocalKind)>,
    out: Vec<Span>,
}

impl<'a> Walker<'a> {
    fn walk(&mut self, node: &'a SyntaxNode, parent: Option<&'a SyntaxNode>) {
        // Outside the requested span, and so is everything below it.
        if node.end < self.from || node.start > self.to {
            return;
        }

        // `~foo` is one name wearing two tokens. Emitting the node whole keeps
        // the `~` the same colour as the rest of it.
        if node.kind == SyntaxKind::EnvVarRef {
            self.push(node.start, node.end, Type::Variable, 0);
            return;
        }

        let opens_scope = is_scope(node.kind);
        if opens_scope {
            // SuperCollider requires declarations at the top of a body, so
            // every name a scope binds is known before any of it is walked.
            self.scopes.push(declared_in(node, self.source));
        }

        // A class's own slots come from the tree above; the ones it inherits
        // are written in another class and often another file, and without
        // them `pattern` in `Pgate` paints as an ordinary variable.
        //
        // Put back on the way out. Classes do not nest, so this is never a
        // stack — but what follows one in the file is not inside it, and a
        // `.sc` file being typed into has loose code after an unbalanced brace
        // constantly. Leaving the last class's slots in place painted those
        // names as properties of a class they are not in.
        let class_scope = matches!(node.kind, SyntaxKind::ClassDef | SyntaxKind::ClassExtension);
        let outer_slots = class_scope.then(|| {
            let inherited = node
                .token_of(SyntaxKind::ClassName)
                .map(|t| {
                    class_slots(self.index, t.text(self.source))
                        .into_iter()
                        .map(|s| (s.var.name.clone(), s.kind()))
                        .collect()
                })
                .unwrap_or_default();
            std::mem::replace(&mut self.slots, inherited)
        });

        for child in &node.children {
            match child {
                Child::Node(n) => self.walk(n, Some(node)),
                Child::Token(t) => self.token(t, node, parent),
            }
        }

        if let Some(outer) = outer_slots {
            self.slots = outer;
        }
        if opens_scope {
            self.scopes.pop();
        }
    }

    fn token(&mut self, token: &Token, parent: &SyntaxNode, grandparent: Option<&SyntaxNode>) {
        if token.start > self.to || token.end < self.from {
            return;
        }
        if let Some((ty, modifiers)) = self.classify(token, parent, grandparent) {
            self.push(token.start, token.end, ty, modifiers);
        }
    }

    fn push(&mut self, start: u32, end: u32, ty: Type, modifiers: u32) {
        if start < end {
            self.out.push(Span {
                start,
                end,
                ty,
                modifiers,
            });
        }
    }

    fn classify(
        &self,
        token: &Token,
        parent: &SyntaxNode,
        grandparent: Option<&SyntaxNode>,
    ) -> Option<(Type, u32)> {
        use SyntaxKind::*;

        // Comments first: they attach to whatever node the parser was building,
        // which can be one of the positions decided below.
        if matches!(token.kind, LineComment | BlockComment) {
            return Some((Type::Comment, 0));
        }

        // A method may be named by an operator — `++ { }` and `< { }` are
        // both legal — so the name position has to be settled before the
        // token's own kind is consulted.
        if parent.kind == MethodDef {
            return self.classify_name(token, parent, grandparent);
        }

        match token.kind {
            // A `Char` is a one-character string literal, and no predefined
            // type is closer.
            String | Char => Some((Type::String, 0)),

            // `\freq`, `'freq'`. Not `string`: a symbol is drawn from an open
            // set of names, used the way an enum member is, and colouring the
            // two alike would throw away a distinction SuperCollider code
            // leans on heavily.
            Symbol => Some((Type::EnumMember, 0)),

            Integer | Float | RadixInteger | HexInteger | Accidental => Some((Type::Number, 0)),
            // `pi` is a number spelled as a word — including the `2pi` form,
            // where the lexer gives two tokens for one literal.
            PiKw => Some((Type::Number, 0)),

            ClassName => Some((Type::Class, class_modifiers(parent))),
            PrimitiveName => Some((Type::Macro, 0)),
            // `_` in `_ + 1`: the argument of the function that sugar builds.
            CurryArg => Some((Type::Parameter, 0)),

            // `freq: 440` names a parameter; the same spelling inside `( … )`
            // names a key of an event. Both are wrapped in a `KeywordArg`, so
            // which one it is comes from the node above that.
            KeywordBinop => match grandparent.map(|g| g.kind) {
                Some(EventLiteral) => Some((Type::Property, 0)),
                _ => Some((Type::Parameter, 0)),
            },

            // `while` is a distinct token only for the generator syntax;
            // everywhere else it is an ordinary name, so both go through the
            // same classification and the keyword reading is the fallback.
            Ident | WhileKw => {
                self.classify_name(token, parent, grandparent)
                    .or(match token.kind {
                        WhileKw => Some((Type::Keyword, 0)),
                        _ => None,
                    })
            }

            k if k.is_keyword() => Some((Type::Keyword, 0)),

            // `^expr` is a return, which is a keyword everywhere it is spelled
            // with letters.
            Caret => Some((Type::Keyword, 0)),

            // Unambiguous operators: these characters have no other job.
            BinOp | LeftArrow | DotDot | Ellipsis | Backtick => Some((Type::Operator, 0)),

            // `=` binds a value to a name in a declaration exactly as it does
            // in an assignment. Only the assignment was listed below, so the
            // same character was an operator in `x = 1` and uncoloured in
            // `var x = 1` two lines away.
            Eq if matches!(parent.kind, VarDef | SlotDef | MultiAssignExpr | Qualifier) => {
                Some((Type::Operator, 0))
            }

            // The rest are operators only in some positions. `|` also delimits
            // an argument list, `+` also opens a class extension, `*` also
            // marks a class method, and `<` also marks a getter.
            Eq | Lt | Gt | Minus | Plus | Star | Pipe | ReadWriteVar => match parent.kind {
                BinaryExpr | UnaryExpr | AssignExpr | SplatArg | IndexRange | ArithSeries => {
                    Some((Type::Operator, 0))
                }
                RwSpec => Some((Type::Modifier, 0)),
                _ => None,
            },

            _ => None,
        }
    }

    /// A token standing where a name can stand. `None` when the position is
    /// not one, which is how `while` falls back to being a keyword.
    fn classify_name(
        &self,
        token: &Token,
        parent: &SyntaxNode,
        grandparent: Option<&SyntaxNode>,
    ) -> Option<(Type, u32)> {
        use SyntaxKind::*;

        match parent.kind {
            // `foo.bar` — the only bare name under a method call is the
            // selector; the receiver is always a node.
            MethodCall => Some((Type::Method, 0)),

            // `bar { … }`, `*bar { … }`, `++ { … }` on a class.
            MethodDef => match method_name(parent) {
                Some(name) if name.start == token.start => Some((Type::Method, DECLARATION)),
                // The `*` that made it class-side.
                _ if token.kind == Star => Some((Type::Modifier, 0)),
                _ => None,
            },

            // `foo(a, b)` is `a.foo(b)`: the head of a call is a selector,
            // however much it is spelled like a variable. This is also what
            // makes `if`, `while` and `case` read as the method calls they
            // are.
            NameRef if is_call_head(parent, grandparent) => Some((Type::Method, 0)),

            NameRef | MultiAssignTargets => Some(self.name_use(token)),

            // `arg a, b = 1`, `var a`, `|a|`, `...rest`. Which of the two the
            // declaration is comes from the list it sits in.
            VarDef | RestArg => {
                let kind = match grandparent.map(|g| g.kind) {
                    Some(ArgDecls) => LocalKind::Argument,
                    _ => LocalKind::Variable,
                };
                let (ty, modifiers) = paint(kind);
                Some((ty, modifiers | DECLARATION))
            }

            // `var <a`, `classvar b`, `const c = 1` on a class.
            SlotDef => {
                let kind = grandparent.map(slot_kind).unwrap_or(LocalKind::InstanceVar);
                let (ty, modifiers) = paint(kind);
                Some((ty, modifiers | DECLARATION))
            }

            // `Array[slot]` — the indexed form's slot name.
            IndexedSlot => Some((Type::Property, DECLARATION)),

            // `x <- xs` and `var x = y` inside a list comprehension both bind
            // a name; `:while` is the one keyword that reaches here.
            Qualifier if token.kind != WhileKw => Some((Type::Variable, DECLARATION)),

            // `a +.x b` — the adverb names how the operator is applied.
            Adverb => Some((Type::Modifier, 0)),

            _ => None,
        }
    }

    /// A name being used rather than declared.
    fn name_use(&self, token: &Token) -> (Type, u32) {
        let name = token.text(self.source);

        // `this`, `thisProcess` and friends are plain identifiers to the
        // lexer and are bound by the compiler, so nothing declares them and a
        // scope lookup would call them ordinary variables.
        if PSEUDO_VARIABLES.contains(&name) {
            return (Type::Keyword, 0);
        }

        match self.lookup(name) {
            Some(kind) => paint(kind),
            // Nothing in scope declares it: an interpreter variable like `s`,
            // or a declaration the user has not typed yet. Either way it is
            // being used as a variable, which is all that is claimed.
            None => (Type::Variable, 0),
        }
    }

    /// The innermost declaration of a name, since that is the one that wins.
    ///
    /// Lexical scopes first: an argument named `pattern` shadows the class
    /// slot of that name, exactly as it does at run time.
    fn lookup(&self, name: &str) -> Option<LocalKind> {
        self.scopes
            .iter()
            .rev()
            .find_map(|frame| frame.iter().find(|l| l.name == name).map(|l| l.kind))
            .or_else(|| {
                self.slots
                    .iter()
                    .find(|(slot, _)| slot == name)
                    .map(|(_, kind)| *kind)
            })
    }
}

/// How a declared name is coloured wherever it appears.
fn paint(kind: LocalKind) -> (Type, u32) {
    match kind {
        LocalKind::Argument => (Type::Parameter, 0),
        LocalKind::Variable => (Type::Variable, 0),
        LocalKind::InstanceVar => (Type::Property, 0),
        LocalKind::ClassVar => (Type::Property, STATIC),
        LocalKind::Constant => (Type::Property, STATIC | READONLY),
    }
}

/// Which of `var`, `classvar` and `const` a class-level declaration is.
///
/// The same split [`crate::scope`] makes, read from the node rather than from
/// the collected names.
fn slot_kind(decl: &SyntaxNode) -> LocalKind {
    if decl.token_of(SyntaxKind::ClassvarKw).is_some() {
        LocalKind::ClassVar
    } else if decl.token_of(SyntaxKind::ConstKw).is_some() {
        LocalKind::Constant
    } else {
        LocalKind::InstanceVar
    }
}

/// A class name is a declaration only where the class is being defined.
/// `+ Foo { }` adds to a class defined elsewhere, and `: Bar` names one.
fn class_modifiers(parent: &SyntaxNode) -> u32 {
    match parent.kind {
        SyntaxKind::ClassDef => DECLARATION,
        _ => 0,
    }
}

/// The token naming a method.
///
/// `*foo { }` is a class method, but a bare `* { }` is an instance method
/// *named* `*` — `Float.sc` and `Symbol.sc` both define one — so a leading
/// star is the class-side marker only when another name follows it.
fn method_name(def: &SyntaxNode) -> Option<Token> {
    let mut names = def
        .children
        .iter()
        .take_while(|c| c.kind() != SyntaxKind::LBrace)
        .filter_map(|c| match c {
            Child::Token(t) if !t.kind.is_trivia() => Some(*t),
            _ => None,
        });

    let first = names.next()?;
    if first.kind == SyntaxKind::Star {
        Some(names.next().unwrap_or(first))
    } else {
        Some(first)
    }
}

/// Whether a name is the callee of a call rather than a value in it.
fn is_call_head(name: &SyntaxNode, call: Option<&SyntaxNode>) -> bool {
    call.is_some_and(|c| {
        c.kind == SyntaxKind::CallExpr
            && c.child_nodes()
                .next()
                .is_some_and(|head| head.start == name.start && head.end == name.end)
    })
}

/// Turn classified spans into the protocol's flat array of deltas.
///
/// Each token is five integers relative to the one before it, so the order
/// they go out in *is* the encoding: get it wrong and colours slide down the
/// file rather than failing.
fn encode(spans: &[Span], doc: &Document, enc: PositionEncoding) -> Vec<SemanticToken> {
    let source = &doc.text;
    let mut out = Vec::with_capacity(spans.len());
    let mut last_line = 0;
    let mut last_start = 0;

    for span in spans {
        for (start, end) in split_lines(source, span.start, span.end) {
            let position = doc.line_index.position(source, start, enc);
            let length = doc.line_index.position(source, end, enc).character - position.character;

            let delta_line = position.line - last_line;
            out.push(SemanticToken {
                delta_line,
                delta_start: if delta_line == 0 {
                    position.character - last_start
                } else {
                    position.character
                },
                length,
                token_type: span.ty as u32,
                token_modifiers_bitset: span.modifiers,
            });

            last_line = position.line;
            last_start = position.character;
        }
    }

    out
}

/// Cut a byte range at every line break in it.
///
/// A client is only required to accept tokens that stay on one line, and
/// `multilineTokenSupport` is the exception rather than the rule. Block
/// comments and the multi-line strings in every `.schelp` file would break
/// that constantly, so they are split here instead of being advertised for.
fn split_lines(source: &str, start: u32, end: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut from = start;

    for (i, byte) in source[start as usize..end as usize].bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let mut stop = start + i as u32;
        // A CRLF file should not colour the carriage return: the line index
        // treats it as outside the line, and the two have to agree.
        if stop > from && source.as_bytes()[stop as usize - 1] == b'\r' {
            stop -= 1;
        }
        if stop > from {
            out.push((from, stop));
        }
        from = start + i as u32 + 1;
    }

    if from < end {
        out.push((from, end));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Position;
    use std::path::Path;

    /// One decoded token: the text it covers, and what it was called.
    #[derive(Debug, PartialEq, Eq)]
    struct Decoded {
        text: String,
        ty: String,
        modifiers: Vec<String>,
    }

    /// Undo the delta encoding, which is the only way to assert on something
    /// a reader can check by eye — and it exercises the encoder rather than
    /// trusting it.
    fn decode(source: &str, tokens: &[SemanticToken]) -> Vec<Decoded> {
        let index = crate::line_index::LineIndex::new(source);
        let legend = legend();
        let (mut line, mut character) = (0, 0);

        tokens
            .iter()
            .map(|t| {
                line += t.delta_line;
                character = if t.delta_line == 0 {
                    character + t.delta_start
                } else {
                    t.delta_start
                };

                let enc = PositionEncoding::Utf16;
                let start = index.offset(source, Position::new(line, character), enc);
                let end = index.offset(source, Position::new(line, character + t.length), enc);

                Decoded {
                    text: source[start as usize..end as usize].to_string(),
                    ty: legend.token_types[t.token_type as usize]
                        .as_str()
                        .to_string(),
                    modifiers: legend
                        .token_modifiers
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| t.token_modifiers_bitset & (1 << i) != 0)
                        .map(|(_, m)| m.as_str().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    fn tokens(source: &str) -> Vec<Decoded> {
        let doc = Document::new(source.to_string(), 1);
        let tokens = semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf16);
        decode(source, &tokens)
    }

    fn script(source: &str) -> Vec<Decoded> {
        let doc = Document::with_mode(source.to_string(), 1, sclang_syntax::Mode::Script);
        let tokens = semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf16);
        decode(source, &tokens)
    }

    /// The type given to the first token whose text matches.
    fn type_of(found: &[Decoded], text: &str) -> Option<String> {
        found.iter().find(|d| d.text == text).map(|d| d.ty.clone())
    }

    /// Every `(text, type)` pair, for asserting on a whole short input.
    fn pairs(found: &[Decoded]) -> Vec<(&str, &str)> {
        found
            .iter()
            .map(|d| (d.text.as_str(), d.ty.as_str()))
            .collect()
    }

    #[test]
    fn legend_matches_types() {
        // The enum's discriminants are the wire values, so a type added to
        // one list and not the other silently mislabels every token after it.
        assert_eq!(TOKEN_TYPES.len(), Type::Macro as usize + 1);
        assert_eq!(TOKEN_TYPES[Type::Class as usize].as_str(), "class");
        assert_eq!(TOKEN_TYPES[Type::Method as usize].as_str(), "method");
        assert_eq!(TOKEN_TYPES[Type::Parameter as usize].as_str(), "parameter");
        assert_eq!(TOKEN_TYPES[Type::Variable as usize].as_str(), "variable");
        assert_eq!(TOKEN_TYPES[Type::Property as usize].as_str(), "property");
        assert_eq!(
            TOKEN_TYPES[Type::EnumMember as usize].as_str(),
            "enumMember"
        );
        assert_eq!(TOKEN_TYPES[Type::Keyword as usize].as_str(), "keyword");
        assert_eq!(TOKEN_TYPES[Type::Modifier as usize].as_str(), "modifier");
        assert_eq!(TOKEN_TYPES[Type::Comment as usize].as_str(), "comment");
        assert_eq!(TOKEN_TYPES[Type::String as usize].as_str(), "string");
        assert_eq!(TOKEN_TYPES[Type::Number as usize].as_str(), "number");
        assert_eq!(TOKEN_TYPES[Type::Operator as usize].as_str(), "operator");
        assert_eq!(TOKEN_TYPES[Type::Macro as usize].as_str(), "macro");
    }

    #[test]
    fn a_class_definition_names_its_parts() {
        let found = tokens("Foo : Bar { }");
        assert_eq!(pairs(&found), vec![("Foo", "class"), ("Bar", "class")]);
        assert_eq!(found[0].modifiers, vec!["declaration"]);
        // `Bar` is named here, not defined here.
        assert!(found[1].modifiers.is_empty());
    }

    #[test]
    fn a_class_extension_declares_nothing() {
        let found = tokens("+ Foo { bar { ^1 } }");
        assert_eq!(type_of(&found, "Foo").as_deref(), Some("class"));
        assert!(found
            .iter()
            .find(|d| d.text == "Foo")
            .unwrap()
            .modifiers
            .is_empty());
        // The method it adds *is* declared here.
        let bar = found.iter().find(|d| d.text == "bar").unwrap();
        assert_eq!(bar.ty, "method");
        assert_eq!(bar.modifiers, vec!["declaration"]);
    }

    #[test]
    fn a_selector_is_not_a_variable() {
        // The whole point: a TextMate grammar sees a lowercase word here and
        // has no way to tell it from a local.
        let found = tokens("Foo { m { ^x.blend(1) } }");
        assert_eq!(type_of(&found, "blend").as_deref(), Some("method"));
    }

    #[test]
    fn the_head_of_a_call_is_a_selector() {
        // `foo(a)` is `a.foo` in sclang, so `foo` is a method — and this is
        // what makes `if` and `case` read as the calls they are.
        let found = script("if (x) { 1 } { 2 }");
        assert_eq!(type_of(&found, "if").as_deref(), Some("method"));
    }

    #[test]
    fn arguments_and_variables_keep_their_kind_where_they_are_used() {
        let found = script("{ |freq| var scaled; scaled = freq }");
        assert_eq!(
            pairs(&found),
            vec![
                ("freq", "parameter"),
                ("var", "keyword"),
                ("scaled", "variable"),
                ("scaled", "variable"),
                ("=", "operator"),
                ("freq", "parameter"),
            ]
        );
        // Only the declarations carry the modifier.
        assert_eq!(found[0].modifiers, vec!["declaration"]);
        assert_eq!(found[2].modifiers, vec!["declaration"]);
        assert!(found[3].modifiers.is_empty());
        assert!(found[5].modifiers.is_empty());
    }

    #[test]
    fn an_inner_declaration_shadows_an_outer_one() {
        // `v` is an argument outside and a variable inside, and the colour
        // has to follow the innermost declaration exactly as dispatch would.
        let found = script("{ |v| { var v; v } }");
        let uses: Vec<_> = found.iter().filter(|d| d.text == "v").collect();
        assert_eq!(uses.len(), 3);
        assert_eq!(uses[0].ty, "parameter");
        assert_eq!(uses[1].ty, "variable");
        assert_eq!(uses[2].ty, "variable");
    }

    #[test]
    fn a_name_leaving_scope_stops_being_a_parameter() {
        // After the block closes, `freq` is just a name again. A walk that
        // forgot to pop its scope would still call this a parameter.
        let found = script("{ |freq| freq }; freq");
        let uses: Vec<_> = found.iter().filter(|d| d.text == "freq").collect();
        assert_eq!(uses[1].ty, "parameter");
        assert_eq!(uses[2].ty, "variable");
    }

    #[test]
    fn class_slots_are_properties() {
        let found = tokens("T { var <count; classvar all; const max = 9; m { ^count } }");
        let slot = |name: &str| found.iter().find(|d| d.text == name).unwrap();
        assert_eq!(slot("count").ty, "property");
        assert_eq!(slot("all").modifiers, vec!["declaration", "static"]);
        assert_eq!(
            slot("max").modifiers,
            vec!["declaration", "readonly", "static"]
        );
        // The getter marker is a modifier, not a less-than.
        assert_eq!(type_of(&found, "<").as_deref(), Some("modifier"));
        // And the use inside the method is the same property.
        let uses: Vec<_> = found.iter().filter(|d| d.text == "count").collect();
        assert_eq!(uses[1].ty, "property");
        assert!(uses[1].modifiers.is_empty());
    }

    #[test]
    fn inherited_slots_do_not_outlive_the_class_body() {
        // The slots of the class being walked come from the index, and used to
        // be left in place on the way out — so a name written after the class
        // was painted as a property of a class it is not in. Loose code after
        // an unbalanced brace is the ordinary state of a file being typed.
        let mut index = SymbolIndex::default();
        index.index_file(Path::new("Base.sc"), "Base : Object { var inherited; }");
        index.index_file(Path::new("Foo.sc"), "Foo : Base { }");

        let source = "Foo : Base { m { ^inherited } }\ninherited.postln;";
        let doc = Document::new(source.to_string(), 1);
        let found = decode(
            source,
            &semantic_tokens(&doc, &index, PositionEncoding::Utf16),
        );

        let uses: Vec<_> = found.iter().filter(|d| d.text == "inherited").collect();
        assert_eq!(uses[0].ty, "property", "inside the class body");
        assert_eq!(uses[1].ty, "variable", "after it");
    }

    #[test]
    fn an_equals_is_an_operator_in_a_declaration_too() {
        // The same character doing the same job: it was coloured in `x = 1`
        // and left alone in `var x = 1` two lines away.
        for source in [
            "T { var alpha = 1; m { ^alpha } }",
            "T { m { var beta = 2; ^beta } }",
        ] {
            assert_eq!(
                type_of(&tokens(source), "=").as_deref(),
                Some("operator"),
                "{source}"
            );
        }
        assert_eq!(
            type_of(&script("#a, b = [1, 2];"), "=").as_deref(),
            Some("operator")
        );
    }

    #[test]
    fn pseudo_variables_are_keywords() {
        let found = tokens("T { m { ^this.foo } }");
        assert_eq!(type_of(&found, "this").as_deref(), Some("keyword"));
    }

    #[test]
    fn literals_keep_symbols_apart_from_strings() {
        let found = script(r#"["hi", \sym, 'quoted', $c, 42, 1.5, 0x1F, pi]"#);
        assert_eq!(
            pairs(&found),
            vec![
                ("\"hi\"", "string"),
                ("\\sym", "enumMember"),
                ("'quoted'", "enumMember"),
                ("$c", "string"),
                ("42", "number"),
                ("1.5", "number"),
                ("0x1F", "number"),
                ("pi", "number"),
            ]
        );
    }

    #[test]
    fn an_environment_variable_is_one_token_including_its_tilde() {
        let found = script("~out.play");
        assert_eq!(
            pairs(&found),
            vec![("~out", "variable"), ("play", "method")]
        );
    }

    #[test]
    fn keyword_arguments_name_parameters_and_event_keys_name_properties() {
        assert_eq!(
            type_of(&script("SinOsc.ar(freq: 440)"), "freq:").as_deref(),
            Some("parameter")
        );
        assert_eq!(
            type_of(&script("(freq: 440)"), "freq:").as_deref(),
            Some("property")
        );
    }

    #[test]
    fn a_star_marks_a_class_method_but_can_also_be_one() {
        let found = tokens("T { *ar { ^1 } }");
        assert_eq!(pairs(&found)[1], ("*", "modifier"));
        assert_eq!(pairs(&found)[2], ("ar", "method"));

        // `* { }` is an instance method *named* `*`, as Float.sc defines.
        let found = tokens("T { * { |that| ^that } }");
        let star = found.iter().find(|d| d.text == "*").unwrap();
        assert_eq!(star.ty, "method");
        assert_eq!(star.modifiers, vec!["declaration"]);
    }

    #[test]
    fn an_operator_method_is_a_method() {
        let found = tokens("T { ++ { |that| ^that } }");
        let plus = found.iter().find(|d| d.text == "++").unwrap();
        assert_eq!(plus.ty, "method");
        assert_eq!(plus.modifiers, vec!["declaration"]);
    }

    #[test]
    fn a_pipe_delimiting_arguments_is_not_an_operator() {
        let found = script("{ |a| a | 2 }");
        let pipes: Vec<_> = found.iter().filter(|d| d.text == "|").collect();
        // Only the infix one is an operator; the two delimiters are silent.
        assert_eq!(pipes.len(), 1);
        assert_eq!(pipes[0].ty, "operator");
    }

    #[test]
    fn while_is_a_keyword_only_where_the_grammar_makes_it_one() {
        // As a selector it is an ordinary name.
        assert_eq!(
            type_of(&script("x.while({ 1 })"), "while").as_deref(),
            Some("method")
        );
        // In a generator clause it is the keyword it is lexed as.
        assert_eq!(
            type_of(&script("{: x, x <- (0..3), :while (x < 2) }"), "while").as_deref(),
            Some("keyword")
        );
    }

    #[test]
    fn a_generator_clause_binds_a_name() {
        let found = script("{: x * 2, x <- (0..3) }");
        let uses: Vec<_> = found.iter().filter(|d| d.text == "x").collect();
        assert_eq!(uses[1].ty, "variable");
        assert_eq!(uses[1].modifiers, vec!["declaration"]);
    }

    #[test]
    fn a_primitive_is_a_macro() {
        let found = tokens("T { m { _Thing_Do; ^this } }");
        assert_eq!(type_of(&found, "_Thing_Do").as_deref(), Some("macro"));
    }

    #[test]
    fn comments_are_emitted_and_split_at_line_breaks() {
        let found = script("// one\n/* two\nthree */\n1");
        assert_eq!(
            pairs(&found),
            vec![
                ("// one", "comment"),
                // The block comment spans two lines, so it goes out as two
                // tokens: the protocol's default forbids one that wraps.
                ("/* two", "comment"),
                ("three */", "comment"),
                ("1", "number"),
            ]
        );
    }

    #[test]
    fn a_multi_line_string_is_split_too() {
        let source = "\"one\ntwo\"";
        let found = script(source);
        assert_eq!(
            pairs(&found),
            vec![("\"one", "string"), ("two\"", "string")]
        );
    }

    #[test]
    fn crlf_line_ends_are_not_coloured() {
        let found = script("/* a\r\nb */");
        assert_eq!(
            pairs(&found),
            vec![("/* a", "comment"), ("b */", "comment")]
        );
    }

    #[test]
    fn every_token_is_encoded_forwards() {
        // The delta encoding has no way to go backwards, so an out-of-order
        // span silently corrupts every token after it.
        let source = "// lead\nFoo : Bar {\n\tclassvar <all;\n\t*new { |n = 4|\n\t\t^super.new.init(n)\n\t}\n\tinit { |n| all = all ++ [n]; ^this }\n}\n";
        let doc = Document::new(source.to_string(), 1);
        let tokens = semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf16);
        for token in &tokens {
            if token.delta_line == 0 {
                // Two tokens on one line may touch but never overlap or
                // reverse; `delta_start` is unsigned, so a reversal would
                // have wrapped rather than failed here.
                assert!(token.length > 0);
            }
        }
        let decoded = decode(source, &tokens);
        // Spot-check that the whole thing lines up: every decoded token's
        // text is the source text at its own range.
        assert_eq!(type_of(&decoded, "super").as_deref(), Some("keyword"));
        assert_eq!(type_of(&decoded, "init").as_deref(), Some("method"));
        assert_eq!(type_of(&decoded, "++").as_deref(), Some("operator"));
    }

    #[test]
    fn non_ascii_positions_use_the_negotiated_encoding() {
        // The emoji is 4 bytes, 2 UTF-16 units. A length in bytes would run
        // the comment token past the end of the line.
        let source = "// 🎛\nFoo { }";
        let doc = Document::new(source.to_string(), 1);
        let utf16 = semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf16);
        assert_eq!(utf16[0].length, 5);
        let utf8 = semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf8);
        assert_eq!(utf8[0].length, 7);
    }

    #[test]
    fn a_range_request_covers_only_that_range() {
        let source = "Foo { }\nBar { }\n";
        let doc = Document::new(source.to_string(), 1);
        let second_line = Range::new(Position::new(1, 0), Position::new(1, 7));
        let tokens = semantic_tokens_range(
            &doc,
            &SymbolIndex::default(),
            second_line,
            PositionEncoding::Utf16,
        );
        assert_eq!(pairs(&decode(source, &tokens)), vec![("Bar", "class")]);
    }

    #[test]
    fn a_range_request_still_resolves_names_from_outside_it() {
        // `freq` is declared on line 0 and used on line 2. Pruning the walk
        // to the range must not prune the scope it is declared in.
        let source = "{ |freq|\n\n\tfreq\n}";
        let doc = Document::new(source.to_string(), 1);
        let third_line = Range::new(Position::new(2, 0), Position::new(2, 5));
        let tokens = semantic_tokens_range(
            &doc,
            &SymbolIndex::default(),
            third_line,
            PositionEncoding::Utf16,
        );
        assert_eq!(pairs(&decode(source, &tokens)), vec![("freq", "parameter")]);
    }

    #[test]
    fn a_half_typed_buffer_still_colours() {
        // The usual state of a file being worked in. The parser recovers, so
        // there is always a tree to walk.
        let found = tokens("Foo : Bar {\n\tm { |freq| ^freq.");
        assert_eq!(type_of(&found, "Foo").as_deref(), Some("class"));
        assert_eq!(type_of(&found, "freq").as_deref(), Some("parameter"));
    }

    #[test]
    fn an_empty_document_produces_nothing() {
        assert!(tokens("").is_empty());
    }

    /// The properties that hold for *any* input, checked the way a client
    /// reads the response.
    ///
    /// These are what the delta encoding makes easy to break and impossible
    /// to see: a wrong offset raises nothing anywhere, it just slides colour
    /// down the file from wherever it went wrong. `examples/token_sweep.rs`
    /// runs the same checks over a real class library and the help-file
    /// corpus, neither of which is in the repository; this keeps them in CI
    /// on inputs that are.
    fn well_formed(source: &str, mode: sclang_syntax::Mode) {
        let doc = Document::with_mode(source.to_string(), 1, mode);
        let legend = legend();
        let index = crate::line_index::LineIndex::new(source);

        // Both encodings: the arithmetic differs, and an input with a `°` or
        // an emoji in a comment only breaks one of them.
        for enc in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            let tokens = semantic_tokens(&doc, &SymbolIndex::default(), enc);
            let (mut line, mut character) = (0, 0);
            let mut previous_end = 0;

            for (i, token) in tokens.iter().enumerate() {
                line += token.delta_line;
                character = if token.delta_line == 0 {
                    character + token.delta_start
                } else {
                    token.delta_start
                };
                let at = format!("token {i} at {line}:{character} ({enc:?}) in {source:?}");

                assert!(token.length > 0, "{at} has zero length");
                assert!(
                    (token.token_type as usize) < legend.token_types.len(),
                    "{at} has a type outside the legend"
                );
                assert!(
                    token.token_modifiers_bitset < (1 << legend.token_modifiers.len()),
                    "{at} has modifier bits outside the legend"
                );

                let start = Position::new(line, character);
                let end = Position::new(line, character + token.length);
                let from = index.offset(source, start, enc);
                let to = index.offset(source, end, enc);

                // `offset` clamps a position past the end of its line, so the
                // round trip is the only thing that notices one.
                assert_eq!(
                    index.position(source, from, enc),
                    start,
                    "{at} starts past the end of its line"
                );
                assert_eq!(
                    index.position(source, to, enc),
                    end,
                    "{at} runs past the end of its line"
                );
                assert!(
                    !source[from as usize..to as usize].contains(['\n', '\r']),
                    "{at} spans a line break"
                );
                assert!(from >= previous_end, "{at} overlaps the token before it");
                previous_end = to;
            }
        }
    }

    #[test]
    fn tokens_are_well_formed_on_awkward_input() {
        // Each of these breaks a different one of the properties when the
        // encoder is wrong, and every one is a shape that occurs in the help
        // files or the class library rather than an invention.
        let scripts = [
            "",
            "\n\n\n",
            "// trailing comment with no newline",
            "/* unterminated",
            "\"unterminated",
            "/* a\nb\nc */ 1",
            "\"one\ntwo\nthree\"",
            "/* a\r\nb */\r\n1\r\n",
            "// 🎛 knob\n~out = 1;",
            "x = \"°ø\" ++ \\sym;",
            "( var a = 1; a.postln; )",
            "{ |freq = 440, ...rest| freq }",
            "SinOsc.ar(freq: 440, mul: 0.5)",
            "(freq: 440, dur: 1)",
            "#a, b = [1, 2];",
            "{: x * 2, x <- (0..3), :while (x < 2) }",
            "a +.x b",
            "[1, 2].sum;",
            "2pi + 16rF700 + 0x1F + 4s50 + $c",
            "SinOsc.ar(440",
            "Foo : Bar { m { |a| ^a.",
            "~",
            ".",
            "}",
        ];
        for source in scripts {
            well_formed(source, sclang_syntax::Mode::Script);
        }

        let classes = [
            "Foo : Bar { }",
            "+ Foo { bar { ^1 } }",
            "Array[slot] : ArrayedCollection { }",
            "T { var <a, >b, <>c; classvar d; const e = 1; }",
            "T { * { |that| ^that } ++ { |that| ^that } *new { ^super.new } }",
            "T { m { _Prim_Do; ^this } }",
            "T {\r\n\tm { ^1 }\r\n}",
            "Foo : Bar { m { |a| ^a.",
        ];
        for source in classes {
            well_formed(source, sclang_syntax::Mode::ClassFile);
        }
    }

    /// The tokens of a script, as the cache would be handed them.
    fn tokens_of(source: &str) -> Vec<SemanticToken> {
        let doc = Document::with_mode(source.to_string(), 1, sclang_syntax::Mode::Script);
        semantic_tokens(&doc, &SymbolIndex::default(), PositionEncoding::Utf16)
    }

    fn five(token: &SemanticToken) -> [u32; 5] {
        [
            token.delta_line,
            token.delta_start,
            token.length,
            token.token_type,
            token.token_modifiers_bitset,
        ]
    }

    /// Apply edits the way a client does: on the flat array of integers, which
    /// is the only place an off-by-five shows up.
    fn apply(previous: &[SemanticToken], edits: &[SemanticTokensEdit]) -> Vec<SemanticToken> {
        let mut flat: Vec<u32> = previous.iter().flat_map(five).collect();
        // Each edit indexes the array as the ones before it left it, so going
        // backwards keeps every index valid. Only ever one is produced here,
        // but a helper that is wrong about this would hide the day that
        // changes.
        for edit in edits.iter().rev() {
            let start = edit.start as usize;
            let end = start + edit.delete_count as usize;
            let data: Vec<u32> = edit.data.iter().flatten().flat_map(five).collect();
            flat.splice(start..end, data);
        }
        // The flat array is always a whole number of tokens; `as_chunks` says
        // so in the type rather than leaving a remainder to ignore.
        flat.as_chunks::<5>()
            .0
            .iter()
            .map(|c| SemanticToken {
                delta_line: c[0],
                delta_start: c[1],
                length: c[2],
                token_type: c[3],
                token_modifiers_bitset: c[4],
            })
            .collect()
    }

    #[test]
    fn edits_reconstruct_the_new_array() {
        // The property the whole feature rests on. A client that applies
        // these to what it has must end up with what a full request would
        // have given it — anything else is colour that silently disagrees
        // with the buffer until something forces a refresh.
        let pairs = [
            ("Foo { }", "Foo { }"),
            ("", "Foo { }"),
            ("Foo { }", ""),
            ("", ""),
            ("Foo { }", "Fooo { }"),
            ("Foo { }", "Bar { }"),
            ("1 + 2", "1 + 2 + 3"),
            ("a.foo; b.bar", "a.foo; c.baz; b.bar"),
            ("// one\n1", "1"),
            ("{ |a| a }", "{ |a, b| a + b }"),
            ("x\ny\nz", "x\n\ny\nz"),
            ("\"one\ntwo\"", "\"one\ntwo\nthree\""),
        ];

        for (before, after) in pairs {
            let previous = tokens_of(before);
            let current = tokens_of(after);
            let edits = edits(&previous, &current);
            assert_eq!(
                apply(&previous, &edits),
                current,
                "{before:?} -> {after:?} did not reconstruct"
            );
        }
    }

    #[test]
    fn an_unchanged_document_produces_no_edits() {
        let tokens = tokens_of("Foo { }");
        assert!(edits(&tokens, &tokens).is_empty());
    }

    #[test]
    fn an_edit_in_the_middle_sends_only_what_changed() {
        // The point of the feature, and the one property a correct-but-whole
        // array replacement would still pass every other test here.
        //
        // It works because the encoding is relative: the first token on a
        // line carries an absolute column, so renaming something on line 100
        // leaves every token from line 101 on byte-identical.
        let before: String = (0..200).map(|i| format!("var x{i} = {i};\n")).collect();
        let after = before.replace("var x100 = 100;", "var xx100 = 100;");

        let previous = tokens_of(&before);
        let current = tokens_of(&after);
        assert!(previous.len() > 500, "{} tokens", previous.len());

        let edits = edits(&previous, &current);
        assert_eq!(edits.len(), 1);
        let sent = edits[0].data.as_ref().unwrap();
        assert!(
            sent.len() <= 3,
            "a one-line rename resent {} of {} tokens",
            sent.len(),
            current.len()
        );
        assert_eq!(apply(&previous, &edits), current);
    }

    #[test]
    fn a_delta_is_measured_against_the_id_the_client_was_given() {
        let uri = Url::parse("file:///delta.scd").unwrap();
        let mut cache = Cache::default();

        let first = cache.full(&uri, tokens_of("Foo { }"));
        let first_id = first
            .result_id
            .clone()
            .expect("a full response is labelled");

        let second = cache.delta(&uri, &first_id, tokens_of("Bar { }"));
        let SemanticTokensFullDeltaResult::TokensDelta(delta) = second else {
            panic!("expected edits against a live id");
        };
        assert_eq!(apply(&first.data, &delta.edits), tokens_of("Bar { }"));

        // Every response gets its own id, so the client cannot diff twice
        // against the same base.
        let second_id = delta.result_id.expect("a delta is labelled too");
        assert_ne!(second_id, first_id);

        // The id that has now been superseded gets the whole array rather
        // than an error: the client may have been talking to a server that
        // has since restarted.
        let stale = cache.delta(&uri, &first_id, tokens_of("Bar { }"));
        assert!(matches!(stale, SemanticTokensFullDeltaResult::Tokens(_)));
    }

    #[test]
    fn closing_a_document_forgets_what_it_was_sent() {
        let uri = Url::parse("file:///closed.scd").unwrap();
        let mut cache = Cache::default();
        let id = cache.full(&uri, tokens_of("Foo { }")).result_id.unwrap();

        cache.forget(&uri);

        let after = cache.delta(&uri, &id, tokens_of("Foo { }"));
        assert!(
            matches!(after, SemanticTokensFullDeltaResult::Tokens(_)),
            "a reopened document has nothing to diff against"
        );
    }

    #[test]
    fn the_guided_tour_is_well_formed() {
        // Real code, committed to the repository, and written to exercise
        // every feature rather than this one.
        let source = include_str!("../../../../editors/vscode/sample.scd");
        well_formed(source, sclang_syntax::Mode::Script);
    }
}
