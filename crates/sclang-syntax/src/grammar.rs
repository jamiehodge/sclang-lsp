//! The grammar, transcribed from `lang/LangSource/Bison/lang11d`.
//!
//! Each function corresponds to a production or a small group of them, and
//! names them in its doc comment so the two can be read side by side. The
//! shape is recursive descent rather than LR, which is what lets error
//! recovery be placed deliberately — sclang's own parser has no `error`
//! productions at all and stops at the first syntax error.
//!
//! One thing makes SuperCollider unusually easy here: **there is no operator
//! precedence**. `lang11d:10` puts every binary operator on a single `%left`
//! level, so `1 + 2 * 3` is 9, not 7, and binary expressions are a flat
//! left-associative chain with no precedence climbing.

use crate::kind::SyntaxKind::{self, *};
use crate::parser::{CompletedMarker, Parser};

/// Tokens that can begin an expression. Used both to decide whether to parse
/// one and as a recovery target.
const EXPR_START: &[SyntaxKind] = &[
    Ident,
    // `name : NAME | WHILE` (lang11d:378)
    WhileKw,
    ClassName,
    Integer,
    Float,
    RadixInteger,
    HexInteger,
    Accidental,
    String,
    Symbol,
    Char,
    TrueKw,
    FalseKw,
    NilKw,
    PiKw,
    CurryArg,
    PrimitiveName,
    LParen,
    LBrace,
    LBracket,
    BeginClosedFunc,
    Hash,
    Tilde,
    Backtick,
    Caret,
    Minus,
    KeywordBinop,
];

/// `root : classes | classextensions | INTERPRET cmdlinecode`
///
/// A file is either class definitions or loose code, and the parser does not
/// need to be told which: a `ClassName {` or `+ ClassName {` at the top level
/// is a class, anything else is an expression.
pub(crate) fn source_file(p: &mut Parser) {
    let m = p.start();
    while !p.at_end() {
        if !p.progressing() {
            p.error_and_bump("parser stalled");
            continue;
        }
        if at_class_def(p) {
            class_def(p);
        } else if p.at(Plus) && p.nth(1) == ClassName {
            class_extension(p);
        } else if p.at_any(EXPR_START) {
            expr_statement(p);
        } else {
            p.error_and_bump(format!("unexpected {:?} at top level", p.current()));
        }
    }
    m.complete(p, SourceFile);
}

/// A class definition starts with a class name followed by `{`, `:` or `[`.
/// Anything else beginning with a class name is an expression (`SinOsc.ar`).
fn at_class_def(p: &Parser) -> bool {
    p.at(ClassName) && matches!(p.nth(1), LBrace | Colon | LBracket)
}

/// ```text
/// classdef : classname superclass '{' classvardecls methods '}'
///          | classname '[' optname ']' superclass '{' classvardecls methods '}'
/// ```
fn class_def(p: &mut Parser) {
    let m = p.start();
    p.expect(ClassName);

    // The indexed form: `Array[slot] : ArrayedCollection`.
    if p.at(LBracket) {
        let slot = p.start();
        p.bump();
        p.eat(Ident); // `optname` is optional
        p.expect(RBracket);
        slot.complete(p, IndexedSlot);
    }

    if p.at(Colon) {
        let sup = p.start();
        p.bump();
        p.expect(ClassName);
        sup.complete(p, SuperClass);
    }

    class_body(p);
    m.complete(p, ClassDef);
}

/// `classextension : '+' classname '{' methods '}'`
fn class_extension(p: &mut Parser) {
    let m = p.start();
    p.expect(Plus);
    p.expect(ClassName);
    class_body(p);
    m.complete(p, ClassExtension);
}

/// `'{' classvardecls methods '}'`, shared by definitions and extensions.
fn class_body(p: &mut Parser) {
    if !p.expect(LBrace) {
        return;
    }
    while !p.at_end() && !p.at(RBrace) {
        if !p.progressing() {
            p.error_and_bump("parser stalled in class body");
            continue;
        }
        match p.current() {
            ClassvarKw | VarKw | ConstKw => class_var_decl(p),
            _ if at_method_def(p) => method_def(p),
            _ => {
                p.error(format!("expected a member, found {:?}", p.current()));
                // Recover at the next member or the closing brace.
                p.recover_until(&[ClassvarKw, VarKw, ConstKw, RBrace]);
            }
        }
    }
    p.expect(RBrace);
}

/// A method definition is a name, or `*` and a name, followed by `{`. The name
/// may be an operator (`lang11d:107`, `binop '{' ...`), which is why `++ { }`
/// and `< { }` are legal.
fn at_method_def(p: &Parser) -> bool {
    fn is_method_name(k: SyntaxKind) -> bool {
        matches!(
            k,
            Ident
                | WhileKw
                | BinOp
                | Lt
                | Gt
                | Plus
                | Minus
                | Star
                | Pipe
                | Eq
                | ReadWriteVar
                | LeftArrow
        )
    }
    // `* { ... }` is an *instance* method named `*` (Float.sc, Symbol.sc),
    // not a class method with a missing name — so only treat a leading `*` as
    // the class-method marker when a name follows it.
    if p.current() == Star && p.nth(1) != LBrace {
        return is_method_name(p.nth(1)) && p.nth(2) == LBrace;
    }
    is_method_name(p.current()) && p.nth(1) == LBrace
}

/// ```text
/// methoddef : name  '{' argdecls funcvardecls primitive methbody '}'
///           | '*' name '{' ... '}'
///           | binop '{' ... '}'
///           | '*' binop '{' ... '}'
/// ```
fn method_def(p: &mut Parser) {
    let m = p.start();
    // Only a `*` with a name after it is the class-method marker; a bare
    // `* { ... }` is an instance method whose name is `*`.
    if p.at(Star) && p.nth(1) != LBrace {
        p.bump();
    }
    p.bump(); // the name, ordinary or operator
    if p.expect(LBrace) {
        function_body(p);
        p.expect(RBrace);
    }
    m.complete(p, MethodDef);
}

/// `classvardecl : CLASSVAR rwslotdeflist ';' | VAR ... | SC_CONST ...`
fn class_var_decl(p: &mut Parser) {
    let m = p.start();
    p.bump(); // classvar | var | const
    loop {
        if !p.progressing() {
            break;
        }
        slot_def(p);
        if !p.eat(Comma) {
            break;
        }
    }
    if !p.eat(Semicolon) {
        p.error("expected ';' after declaration");
        p.recover_until(&[Semicolon, ClassvarKw, VarKw, ConstKw, RBrace]);
        p.eat(Semicolon);
    }
    m.complete(p, ClassVarDecl);
}

/// `rwslotdef : rwspec name | rwspec name '=' slotliteral`
///
/// `rwspec` is the `<`, `>` or `<>` getter/setter marker.
fn slot_def(p: &mut Parser) {
    let m = p.start();
    if p.at_any(&[Lt, Gt, ReadWriteVar]) {
        let spec = p.start();
        p.bump();
        spec.complete(p, RwSpec);
    }
    if !p.eat(Ident) {
        p.error("expected a variable name");
        m.abandon(p);
        return;
    }
    if p.eat(Eq) {
        expr(p);
    }
    m.complete(p, SlotDef);
}

/// `argdecls funcvardecls primitive methbody` — the inside of any `{ }`.
fn function_body(p: &mut Parser) {
    arg_decls(p);
    while p.at(VarKw) {
        if !p.progressing() {
            break;
        }
        var_decls(p);
    }
    // `primitive : | primname optsemi`
    if p.at(PrimitiveName) {
        let m = p.start();
        p.bump();
        p.eat(Semicolon);
        m.complete(p, Primitive);
    }
    expr_seq(p, &[RBrace]);
}

/// ```text
/// argdecls : | ARG vardeflist ';' | ARG vardeflist0 ELLIPSIS name ';'
///          | '|' slotdeflist '|' | '|' slotdeflist0 ELLIPSIS name '|'
/// ```
fn arg_decls(p: &mut Parser) {
    if p.at(ArgKw) {
        let m = p.start();
        p.bump();
        var_def_list(p, &[Semicolon]);
        if !p.eat(Semicolon) {
            p.error("expected ';' after argument list");
            p.recover_until(&[Semicolon, RBrace]);
            p.eat(Semicolon);
        }
        m.complete(p, ArgDecls);
    } else if p.at(Pipe) {
        let m = p.start();
        p.bump();
        var_def_list(p, &[Pipe]);
        if !p.eat(Pipe) {
            p.error("expected '|' to close argument list");
            p.recover_until(&[Pipe, RBrace]);
            p.eat(Pipe);
        }
        m.complete(p, ArgDecls);
    }
}

/// `slotdef : name optequal slotliteral` — `optequal` really is optional, so
/// `|range -1|` declares `range` with a default of `-1`.
///
/// Only a literal (optionally negated) counts, which is what keeps `|a b|`
/// reading as two argument names rather than a name and a default.
fn at_bare_default(p: &Parser) -> bool {
    let k = if p.current() == Minus {
        p.nth(1)
    } else {
        p.current()
    };
    matches!(
        k,
        Integer
            | Float
            | RadixInteger
            | HexInteger
            | Accidental
            | String
            | Symbol
            | Char
            | TrueKw
            | FalseKw
            | NilKw
            | PiKw
            | Hash
    )
}

/// `funcvardecl : VAR vardeflist ';'`
fn var_decls(p: &mut Parser) {
    let m = p.start();
    p.expect(VarKw);
    var_def_list(p, &[Semicolon]);
    if !p.eat(Semicolon) {
        p.error("expected ';' after variable declaration");
        p.recover_until(&[Semicolon, RBrace]);
        p.eat(Semicolon);
    }
    m.complete(p, VarDecls);
}

/// A comma-separated list of `name` or `name = default`, optionally ending
/// with `...rest`.
fn var_def_list(p: &mut Parser, terminators: &[SyntaxKind]) {
    loop {
        if !p.progressing() || p.at_end() || p.at_any(terminators) {
            break;
        }
        if p.at(Ellipsis) {
            let m = p.start();
            p.bump();
            p.expect(Ident);
            m.complete(p, RestArg);
            break;
        }
        let pipe_form = terminators.contains(&Pipe);
        let m = p.start();
        if !p.eat(Ident) {
            p.error("expected an argument name");
            m.abandon(p);
            p.recover_until(terminators);
            break;
        }
        // `slotdef : name | name optequal slotliteral | name optequal '(' exprseq ')'`
        // The `=` is optional, and a default may be parenthesised —
        // `arg dir, overwrite(true);` is the same as `overwrite = true`.
        if p.at(LParen) {
            p.bump();
            expr(p);
            p.expect(RParen);
        } else if p.eat(Eq) || (pipe_form && at_bare_default(p)) {
            // Inside `| ... |` a bare binary expression would swallow the
            // closing pipe, since `|` is also an operator. Take only a primary
            // there, which is how sclang's grammar scopes it too.
            if pipe_form {
                // `min = -90` is common, so a leading unary minus still has to
                // be taken even though a full binary expression cannot be.
                if p.at(Minus) {
                    let neg = p.start();
                    p.bump();
                    primary_expr(p);
                    neg.complete(p, UnaryExpr);
                } else {
                    primary_expr(p);
                }
            } else {
                expr(p);
            }
        }
        m.complete(p, VarDef);
        // `arg a ...rest;` has no comma before the ellipsis.
        if p.at(Ellipsis) {
            continue;
        }
        // `slotdeflist : slotdef | slotdeflist optcomma slotdef` — in the pipe
        // form the comma is optional, so `|a b|` is as valid as `|a, b|`.
        if !p.eat(Comma) && !(pipe_form && p.at(Ident)) {
            break;
        }
    }
}

/// `exprseq : exprn optsemi`, i.e. `;`-separated expressions.
fn expr_seq(p: &mut Parser, terminators: &[SyntaxKind]) {
    let m = p.start();
    while !p.at_end() && !p.at_any(terminators) {
        if !p.progressing() {
            p.error_and_bump("parser stalled in expression sequence");
            continue;
        }
        let before = p.current_range();
        expr_statement(p);
        // If nothing was consumed, force progress rather than spin.
        if p.current_range() == before && !p.at_end() {
            p.error_and_bump(format!("unexpected {:?}", p.current()));
        }
    }
    m.complete(p, ExprSeq);
}

/// One expression, plus its optional `;`. `^expr` is a return.
fn expr_statement(p: &mut Parser) {
    if p.at(Caret) {
        let m = p.start();
        p.bump();
        if expr(p).is_none() {
            p.error(format!(
                "expected an expression after '^', found {:?}",
                p.current()
            ));
        }
        p.eat(Semicolon);
        m.complete(p, ReturnStmt);
        return;
    }
    if !p.at_any(EXPR_START) {
        p.error_and_bump(format!("expected an expression, found {:?}", p.current()));
        return;
    }
    expr(p);
    p.eat(Semicolon);
}

/// ```text
/// expr : expr1 | expr binop2 adverb expr | name '=' expr | ...
/// ```
///
/// A flat left-associative chain, since every binary operator shares one
/// precedence level.
pub(crate) fn expr(p: &mut Parser) -> Option<CompletedMarker> {
    let mut lhs = unary_expr(p)?;

    loop {
        if !p.progressing() {
            break;
        }
        match p.current() {
            // Assignment. `%right '='` in lang11d:9, so recurse on the right.
            Eq => {
                let m = lhs.precede(p);
                p.bump();
                expr(p);
                lhs = m.complete(p, AssignExpr);
            }
            // Any binary operator, with an optional adverb (`a +.x b`).
            k if is_binary_op(k) => {
                let m = lhs.precede(p);
                p.bump();
                adverb(p);
                if unary_expr(p).is_none() {
                    p.error("expected an expression after operator");
                }
                lhs = m.complete(p, BinaryExpr);
            }
            // `key: value` in argument position is handled by arg_list; at
            // expression level it is a binary selector call.
            KeywordBinop => {
                let m = lhs.precede(p);
                p.bump();
                if unary_expr(p).is_none() {
                    p.error("expected an expression after selector");
                }
                lhs = m.complete(p, BinaryExpr);
            }
            _ => break,
        }
    }
    Some(lhs)
}

fn is_binary_op(k: SyntaxKind) -> bool {
    matches!(
        k,
        BinOp | Plus | Minus | Star | Lt | Gt | Pipe | ReadWriteVar | LeftArrow
    )
}

/// `adverb : | '.' name | '.' integer | '.' '(' exprseq ')'` (`lang11d:1134`).
///
/// Only valid directly after a binary operator, which is why it lives here and
/// not in the postfix chain.
fn adverb(p: &mut Parser) {
    if !p.at(Dot) {
        return;
    }
    if !matches!(p.nth(1), Ident | Integer | LParen) {
        return;
    }
    let m = p.start();
    p.bump(); // '.'
    match p.current() {
        Ident | Integer => p.bump(),
        LParen => {
            p.bump();
            expr(p);
            p.expect(RParen);
        }
        _ => {}
    }
    m.complete(p, Adverb);
}

/// `'-' expr %prec UMINUS`, and `` '`' expr ``.
fn unary_expr(p: &mut Parser) -> Option<CompletedMarker> {
    match p.current() {
        Minus => {
            let m = p.start();
            p.bump();
            unary_expr(p);
            Some(m.complete(p, UnaryExpr))
        }
        Backtick => {
            let m = p.start();
            p.bump();
            unary_expr(p);
            Some(m.complete(p, RefExpr))
        }
        _ => postfix_expr(p),
    }
}

/// A primary expression followed by any number of `.name(...)`, `[i]`, `(...)`
/// or trailing blocks.
fn postfix_expr(p: &mut Parser) -> Option<CompletedMarker> {
    let mut expr = primary_expr(p)?;
    loop {
        if !p.progressing() {
            break;
        }
        match p.current() {
            // `.name`, `.name(args)`, or `.[i]`
            Dot => {
                let m = expr.precede(p);
                p.bump();
                if p.at(LBracket) {
                    p.bump();
                    index_args(p);
                    p.expect(RBracket);
                    expr = m.complete(p, DotIndexExpr);
                } else if p.at(LParen) {
                    // `f.(args)` is shorthand for `f.value(args)`.
                    arg_list(p);
                    expr = m.complete(p, MethodCall);
                } else {
                    if !p.eat(Ident) && !p.eat(WhileKw) {
                        p.error("expected a method name after '.'");
                    }
                    if p.at(LParen) || p.at(LBrace) {
                        arg_list(p);
                    }
                    expr = m.complete(p, MethodCall);
                }
            }
            // `foo(args)` or `foo { }`
            LParen | LBrace | BeginClosedFunc => {
                let m = expr.precede(p);
                arg_list(p);
                expr = m.complete(p, CallExpr);
            }
            // `a[i]`, and the subrange forms `a[i..j]`, `a[i..]`, `a[..j]`.
            LBracket => {
                let m = expr.precede(p);
                p.bump();
                index_args(p);
                if !p.eat(RBracket) {
                    p.error("expected ']'");
                    p.recover_until(&[RBracket, Semicolon, RBrace]);
                    p.eat(RBracket);
                }
                expr = m.complete(p, IndexExpr);
            }
            _ => break,
        }
    }
    Some(expr)
}

/// `'(' arglist1 optkeyarglist ')'` and/or one or more trailing blocks.
fn arg_list(p: &mut Parser) {
    let m = p.start();
    if p.at(LParen) {
        p.bump();
        arg_list_items(p, RParen);
        if !p.eat(RParen) {
            p.error("expected ')'");
            p.recover_until(&[RParen, Semicolon, RBrace]);
            p.eat(RParen);
        }
    }
    // `SomeClass { }` and `foo(1) { } { }` are both legal.
    while p.at(LBrace) || p.at(BeginClosedFunc) {
        if !p.progressing() {
            break;
        }
        function_block(p);
    }
    m.complete(p, ArgList);
}

/// The inside of `[ ... ]` in an index position.
///
/// ```text
/// valrangex1 : expr1 '[' arglist1 DOTDOT ']'
///            | expr1 '[' DOTDOT exprseq ']'
///            | expr1 '[' arglist1 DOTDOT exprseq ']'
/// ```
///
/// So `a[1..]`, `a[..2]` and `a[1..2]` are all subranges, and anything without
/// a `..` is an ordinary index list.
fn index_args(p: &mut Parser) {
    // `a[..n]`
    if p.at(DotDot) {
        let m = p.start();
        p.bump();
        if p.at_any(EXPR_START) {
            expr(p);
        }
        m.complete(p, IndexRange);
        return;
    }
    if p.at(RBracket) {
        return;
    }

    let m = p.start();
    arg_list_items_until(p, &[RBracket, DotDot]);
    if p.at(DotDot) {
        p.bump();
        // `a[1..]` has no upper bound.
        if p.at_any(EXPR_START) {
            expr(p);
        }
        m.complete(p, IndexRange);
    } else {
        m.abandon(p);
    }
}

/// Comma-separated arguments, stopping at any of `terminators`.
fn arg_list_items_until(p: &mut Parser, terminators: &[SyntaxKind]) {
    while !p.at_end() && !p.at_any(terminators) {
        if !p.progressing() {
            break;
        }
        if p.at(Star) {
            let m = p.start();
            p.bump();
            expr(p);
            m.complete(p, SplatArg);
        } else if p.at(KeywordBinop) {
            let m = p.start();
            p.bump();
            expr(p);
            m.complete(p, KeywordArg);
        } else if p.at_any(EXPR_START) {
            expr(p);
        } else {
            p.error(format!("unexpected {:?} in index", p.current()));
            p.recover_until(terminators);
            break;
        }
        if !p.eat(Comma) {
            break;
        }
    }
}

/// Comma-separated arguments, each optionally `key: value`.
fn arg_list_items(p: &mut Parser, terminator: SyntaxKind) {
    while !p.at_end() && !p.at(terminator) {
        if !p.progressing() {
            break;
        }
        if p.at(KeywordBinop) {
            let m = p.start();
            p.bump();
            expr(p);
            m.complete(p, KeywordArg);
        } else if p.at(Star) {
            // `arglistv1 : '*' exprseq` — array expansion, as in `f(*args)`.
            let m = p.start();
            p.bump();
            expr(p);
            m.complete(p, SplatArg);
        } else if p.at_any(EXPR_START) {
            expr(p);
        } else {
            p.error(format!("unexpected {:?} in argument list", p.current()));
            p.recover_until(&[Comma, terminator]);
        }
        // A trailing `;` shows up inside call parentheses in real code.
        p.eat(Semicolon);
        if !p.eat(Comma) {
            break;
        }
    }
}

/// `block : '{' argdecls funcvardecls funcbody '}'`, and the `#{ }` closed
/// form.
fn function_block(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(); // '{' or '#{'
    function_body(p);
    p.expect(RBrace);
    m.complete(p, FunctionBlock)
}

/// Literals, names, and the bracketed forms.
fn primary_expr(p: &mut Parser) -> Option<CompletedMarker> {
    let kind = p.current();
    let done = match kind {
        Integer | Float | RadixInteger | HexInteger | Accidental | String | Symbol | Char
        | TrueKw | FalseKw | NilKw | PiKw => {
            let m = p.start();
            p.bump();
            // Adjacent string literals concatenate; the lexer emits one token
            // per segment, so the join happens here.
            if kind == String {
                while p.at(String) {
                    if !p.progressing() {
                        break;
                    }
                    p.bump();
                }
            }
            // `floatp : floatr pie | integer pie | pie` — `0.5pi` and `2pi`
            // are single literals, lexed as two tokens.
            if matches!(kind, Integer | Float | RadixInteger | HexInteger) {
                p.eat(PiKw);
            }
            m.complete(p, Literal)
        }
        // `name : NAME | WHILE` — `while` is only a distinct token for the
        // generator syntax; everywhere else it is an ordinary identifier.
        Ident | WhileKw => {
            let m = p.start();
            p.bump();
            m.complete(p, NameRef)
        }
        ClassName => {
            let m = p.start();
            p.bump();
            m.complete(p, ClassRef)
        }
        CurryArg | PrimitiveName => {
            let m = p.start();
            p.bump();
            m.complete(p, NameRef)
        }
        // `~name`
        Tilde => {
            let m = p.start();
            p.bump();
            if !p.eat(Ident) {
                p.error("expected a name after '~'");
            }
            m.complete(p, EnvVarRef)
        }
        LBrace | BeginClosedFunc => function_block(p),
        LParen => paren_or_series(p),
        LBracket => collection(p),
        // `#[1, 2]` literal array, or `#a, b = c` destructuring.
        Hash => {
            if p.nth(1) == LBracket {
                let m = p.start();
                p.bump();
                p.bump();
                arg_list_items(p, RBracket);
                p.expect(RBracket);
                m.complete(p, LiteralList)
            } else {
                multi_assign(p)
            }
        }
        _ => return None,
    };
    Some(done)
}

/// `'#' mavars '=' expr` — destructuring assignment.
fn multi_assign(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(); // '#'
    let targets = p.start();
    loop {
        if !p.progressing() {
            break;
        }
        if p.at(Ellipsis) {
            p.bump();
            p.expect(Ident);
            break;
        }
        if !p.eat(Ident) {
            p.error("expected a name in destructuring assignment");
            break;
        }
        // `#a ...rest = x` — no comma before the rest element.
        if p.at(Ellipsis) {
            continue;
        }
        if !p.eat(Comma) {
            break;
        }
    }
    targets.complete(p, MultiAssignTargets);
    p.expect(Eq);
    expr(p);
    m.complete(p, MultiAssignExpr)
}

/// `( ... )` is a parenthesised expression, an arithmetic series, or an event
/// literal, distinguished by what is inside.
fn paren_or_series(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(); // '('

    // `(..n)` — a series with no lower bound.
    if p.at(DotDot) {
        p.bump();
        expr(p);
        p.expect(RParen);
        return m.complete(p, ArithSeries);
    }

    // `(key: value, ...)` — an event literal.
    if p.at(KeywordBinop) {
        while !p.at_end() && !p.at(RParen) {
            if !p.progressing() {
                break;
            }
            if p.at(KeywordBinop) {
                let kv = p.start();
                p.bump();
                expr(p);
                p.eat(Semicolon);
                kv.complete(p, KeywordArg);
            } else if p.at_any(EXPR_START) {
                // `dictslotdef : exprseq ':' exprseq` — the key may be any
                // expression, as in `(0: 0, 1: 1)`.
                let kv = p.start();
                expr(p);
                if p.eat(Colon) {
                    expr(p);
                    // `exprseq : exprn optsemi` — the value may end with `;`.
                    p.eat(Semicolon);
                    kv.complete(p, KeywordArg);
                } else {
                    kv.abandon(p);
                }
            } else {
                p.error(format!("unexpected {:?} in event literal", p.current()));
                p.recover_until(&[Comma, RParen]);
            }
            if !p.eat(Comma) {
                break;
            }
        }
        p.expect(RParen);
        return m.complete(p, EventLiteral);
    }

    if p.at(RParen) {
        p.bump();
        return m.complete(p, ParenExpr);
    }

    let first = expr(p);

    // `(a, b .. c)` — the step form.
    if p.at(Comma) {
        p.bump();
        expr(p);
        if p.eat(DotDot) {
            if p.at_any(EXPR_START) {
                expr(p);
            }
            p.expect(RParen);
            return m.complete(p, ArithSeries);
        }
        // Not a series after all: a parenthesised sequence.
        while p.eat(Comma) {
            if !p.progressing() {
                break;
            }
            expr(p);
        }
        p.expect(RParen);
        return m.complete(p, ParenExpr);
    }

    // `(0: 0, ...)` — an event literal whose first key is not an identifier.
    if p.at(Colon) {
        // The key was parsed before we knew it was one, so wrap it and the
        // value together retroactively.
        if let Some(key) = first {
            let kv = key.precede(p);
            p.bump();
            expr(p);
            // `exprseq : exprn optsemi` — the value may end with `;`.
            p.eat(Semicolon);
            kv.complete(p, KeywordArg);
        } else {
            p.bump();
            expr(p);
            p.eat(Semicolon);
        }
        while p.eat(Comma) {
            if !p.progressing() {
                break;
            }
            if p.at(KeywordBinop) {
                let kv = p.start();
                p.bump();
                expr(p);
                p.eat(Semicolon);
                kv.complete(p, KeywordArg);
            } else if p.at_any(EXPR_START) {
                let kv = p.start();
                expr(p);
                if p.eat(Colon) {
                    expr(p);
                    // `exprseq : exprn optsemi` — the value may end with `;`.
                    p.eat(Semicolon);
                    kv.complete(p, KeywordArg);
                } else {
                    kv.abandon(p);
                }
            } else {
                break;
            }
        }
        p.expect(RParen);
        return m.complete(p, EventLiteral);
    }

    // `(a .. b)`
    if p.eat(DotDot) {
        if p.at_any(EXPR_START) {
            expr(p);
        }
        p.expect(RParen);
        return m.complete(p, ArithSeries);
    }

    // A plain parenthesised expression, possibly a `;`-separated block.
    while p.at(Semicolon) {
        if !p.progressing() {
            break;
        }
        p.bump();
        if p.at_any(EXPR_START) {
            expr(p);
        }
    }
    if !p.eat(RParen) {
        p.error("expected ')'");
        p.recover_until(&[RParen, Semicolon, RBrace]);
        p.eat(RParen);
    }
    m.complete(p, ParenExpr)
}

/// `[a, b, c]`. A preceding class name (`Set[...]`) is handled by the postfix
/// chain, which sees it as an index on a class reference.
fn collection(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(); // '['
    arg_list_items(p, RBracket);
    if !p.eat(RBracket) {
        p.error("expected ']'");
        p.recover_until(&[RBracket, Semicolon, RBrace]);
        p.eat(RBracket);
    }
    m.complete(p, Collection)
}
