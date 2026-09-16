//! Reading meaning out of the tree at a point.
//!
//! Everything the editor asks about a cursor — what is under it, what may
//! follow it — resolves here, against the syntax tree and the symbol index.
//! No part of this consults a running sclang, which is what lets it answer
//! while the class library is broken or sclang is not installed at all.

use sclang_index::{MethodKind, SymbolIndex};
use sclang_syntax::{Child, SyntaxKind, SyntaxNode, Token};

/// A token, with the chain of nodes that contains it.
///
/// The ancestors are what give a token its role: an `Ident` is a method
/// selector, a variable reference or a method name depending only on where it
/// sits.
#[derive(Debug, Clone)]
pub struct TokenPath<'a> {
    pub token: Token,
    /// Root first, innermost node last.
    pub ancestors: Vec<&'a SyntaxNode>,
}

impl<'a> TokenPath<'a> {
    pub fn parent(&self) -> Option<&'a SyntaxNode> {
        self.ancestors.last().copied()
    }

    pub fn text(&self, source: &'a str) -> &'a str {
        self.token.text(source)
    }
}

/// Every node containing `offset`, outermost first.
///
/// The end is inclusive, because a cursor at the end of an unclosed block is
/// still inside it — which is the usual state of a buffer being typed into.
pub fn ancestors_at(root: &SyntaxNode, offset: u32) -> Vec<&SyntaxNode> {
    let mut out = Vec::new();
    let mut node = root;
    if offset < node.start || offset > node.end {
        return out;
    }
    loop {
        out.push(node);
        let next = node
            .child_nodes()
            .find(|n| n.start <= offset && offset <= n.end);
        match next {
            Some(child) => node = child,
            None => return out,
        }
    }
}

/// Move an offset back over trivia to the end of the code before it.
///
/// Trailing whitespace belongs to whichever node the parser was building when
/// it ran out of code, which is usually an ancestor of the interesting one:
/// with the cursor after `SinOsc.ar(440, ` the argument list ends at the comma
/// and only the file contains the offset. Stepping back lands inside the call
/// again, which is where the user plainly is.
///
/// An offset already inside a real token is returned untouched.
pub fn skip_trivia_back(root: &SyntaxNode, offset: u32) -> u32 {
    let mut last_end = None;
    let mut found = None;

    visit_tokens(root, &mut |token| {
        if found.is_some() || token.kind.is_trivia() {
            return;
        }
        if token.start < offset && offset <= token.end {
            found = Some(offset);
        } else if token.end <= offset {
            last_end = Some(token.end);
        }
    });

    found.or(last_end).unwrap_or(offset)
}

/// Every token in a tree, in source order.
pub fn visit_tokens(node: &SyntaxNode, f: &mut impl FnMut(&Token)) {
    for child in &node.children {
        match child {
            Child::Node(n) => visit_tokens(n, f),
            Child::Token(t) => f(t),
        }
    }
}

/// Which token an offset should be attributed to when it falls on a boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    /// The token containing the offset: `start <= offset < end`. What hover
    /// and goto want, since the cursor sits *on* a word.
    Inside,
    /// The token ending at the offset: `start < offset <= end`. What
    /// completion wants, since the cursor sits *after* what was typed.
    Before,
}

/// Find the token at `offset`, with its ancestors.
pub fn token_at(root: &SyntaxNode, offset: u32, bias: Bias) -> Option<TokenPath<'_>> {
    let mut ancestors = Vec::new();
    descend(root, offset, bias, &mut ancestors)
}

fn descend<'a>(
    node: &'a SyntaxNode,
    offset: u32,
    bias: Bias,
    ancestors: &mut Vec<&'a SyntaxNode>,
) -> Option<TokenPath<'a>> {
    ancestors.push(node);
    for child in &node.children {
        let (start, end) = child.range();
        let hit = match bias {
            Bias::Inside => start <= offset && offset < end,
            Bias::Before => start < offset && offset <= end,
        };
        if !hit {
            continue;
        }
        match child {
            Child::Node(n) => {
                if let Some(found) = descend(n, offset, bias, ancestors) {
                    return Some(found);
                }
            }
            Child::Token(t) => {
                return Some(TokenPath {
                    token: *t,
                    ancestors: ancestors.clone(),
                })
            }
        }
    }
    ancestors.pop();
    None
}

/// What a `.` is being sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// A literal class name: `SinOsc.ar`. A class-side send.
    Class(String),
    /// An instance of a known class: `Pbind(...).play`, `"foo".reverse`.
    ///
    /// Not inference — see [`instance_class`] for what qualifies and how
    /// certain each case is.
    Instance(String),
    /// Anything else. SuperCollider is dynamically typed and this server does
    /// no inference, so the honest answer is every class defining the
    /// selector.
    Unknown,
}

/// The class an expression is known to produce, where that is knowable without
/// inferring anything.
///
/// Two kinds of certainty here, and they are worth keeping apart when deciding
/// what to do with the answer.
///
/// **The grammar decides it.** A string literal is a `String`, `[1, 2]` is an
/// `Array`, `{ }` is a `Function`. Nothing can make these wrong.
///
/// **Convention decides it.** `Foo(...)` is `Foo.new(...)` — that desugaring is
/// the language's, not a guess — and `*new` returns an instance of the class it
/// was sent to, almost always via `super.new` or `super.newCopyArgs`. A class
/// is free to return something else, and a few do, so this is overwhelmingly
/// right rather than guaranteed. It is used where a wrong answer costs a
/// missing completion or a wrong jump, and deliberately not where one would
/// render as though it were in the source.
pub fn instance_class(node: &SyntaxNode, source: &str) -> Option<String> {
    match node.kind {
        // `Foo(...)`, which the parser leaves as a call on a class reference.
        SyntaxKind::CallExpr => class_named(node, source),

        // `Foo.new(...)`, spelled out. Any other selector is unknown: only
        // `new` has the convention behind it.
        SyntaxKind::MethodCall => {
            let selector = node.token_of(SyntaxKind::Ident)?;
            if selector.text(source) == "new" {
                class_named(node, source)
            } else {
                None
            }
        }

        SyntaxKind::FunctionBlock => Some("Function".to_string()),
        SyntaxKind::EventLiteral => Some("Event".to_string()),

        // `Set[...]` names its own class; a bare `[...]` is an Array.
        SyntaxKind::Collection => {
            Some(class_named(node, source).unwrap_or_else(|| "Array".to_string()))
        }
        SyntaxKind::LiteralList => Some("Array".to_string()),

        SyntaxKind::Literal => {
            let token = node.child_tokens().find(|t| t.kind.is_literal())?;
            Some(
                match token.kind {
                    SyntaxKind::String => "String",
                    SyntaxKind::Integer | SyntaxKind::RadixInteger | SyntaxKind::HexInteger => {
                        "Integer"
                    }
                    SyntaxKind::Float => "Float",
                    SyntaxKind::Symbol => "Symbol",
                    SyntaxKind::Char => "Char",
                    // An accidental like `4s` is a degree, not a plain number,
                    // and pinning it down is not worth being wrong about.
                    _ => return None,
                }
                .to_string(),
            )
        }

        _ => None,
    }
}

/// The class name a node is built on: `Foo(...)`, `Foo.new`, `Set[...]`.
fn class_named(node: &SyntaxNode, source: &str) -> Option<String> {
    if let Some(reference) = node.child_of(SyntaxKind::ClassRef) {
        if let Some(name) = reference.token_of(SyntaxKind::ClassName) {
            return Some(name.text(source).to_string());
        }
    }
    node.token_of(SyntaxKind::ClassName)
        .map(|t| t.text(source).to_string())
}

/// The receiver of the method call a `.` token belongs to.
fn receiver_of(call: &SyntaxNode, dot_start: u32, source: &str) -> Receiver {
    // The receiver is the last node before the dot. Taking it positionally
    // rather than by index keeps this correct for `a.b.c`, where the receiver
    // of the second dot is the whole `a.b` call.
    let before = call
        .children
        .iter()
        .rev()
        .find(|c| c.range().1 <= dot_start);

    receiver_from(before, source)
}

/// Turn the node left of a dot into a receiver.
pub(crate) fn receiver_from(node: Option<&Child>, source: &str) -> Receiver {
    let Some(Child::Node(n)) = node else {
        return Receiver::Unknown;
    };

    if n.kind == SyntaxKind::ClassRef {
        return n
            .token_of(SyntaxKind::ClassName)
            .map(|t| Receiver::Class(t.text(source).to_string()))
            .unwrap_or(Receiver::Unknown);
    }

    instance_class(n, source)
        .map(Receiver::Instance)
        .unwrap_or(Receiver::Unknown)
}

/// A call the cursor is inside: its argument list and what it dispatches to.
#[derive(Debug, Clone)]
pub struct CallSite<'a> {
    pub arg_list: &'a SyntaxNode,
    pub selector: String,
    pub receiver: Receiver,
    /// The cursor, moved back over any trivia. Consumers that count commas or
    /// test containment must use this rather than the raw offset, or a cursor
    /// sitting in the space after a comma falls outside the call.
    pub offset: u32,
}

/// The innermost call containing `offset`.
///
/// Innermost so that a call written inside an argument describes itself rather
/// than the call it sits in. Shared by signature help, keyword-argument
/// completion and inlay hints, which all need the same question answered.
pub fn call_at<'a>(root: &'a SyntaxNode, source: &str, offset: u32) -> Option<CallSite<'a>> {
    // A cursor in the space after a comma is still inside the call.
    let offset = skip_trivia_back(root, offset);
    let path = ancestors_at(root, offset);
    let position = path.iter().rposition(|n| n.kind == SyntaxKind::ArgList)?;
    let arg_list = path[position];
    let call = path.get(position.checked_sub(1)?)?;
    let (selector, receiver) = callee(call, arg_list, source)?;
    Some(CallSite {
        arg_list,
        selector,
        receiver,
        offset,
    })
}

/// The selector a call invokes, and what it is sent to.
pub fn callee(
    call: &SyntaxNode,
    arg_list: &SyntaxNode,
    source: &str,
) -> Option<(String, Receiver)> {
    match call.kind {
        // `receiver.selector(...)`
        SyntaxKind::MethodCall => {
            let dot = call
                .child_tokens()
                .filter(|t| t.kind == SyntaxKind::Dot && t.end <= arg_list.start)
                .last()?;
            let selector = call
                .child_tokens()
                .find(|t| t.kind == SyntaxKind::Ident && t.start >= dot.end)?;
            Some((
                selector.text(source).to_string(),
                receiver_of(call, dot.start, source),
            ))
        }

        SyntaxKind::CallExpr => {
            let head = call.child_nodes().next()?;
            match head.kind {
                // `Point(1, 2)` is `Point.new(1, 2)`.
                SyntaxKind::ClassRef => {
                    let class = head.token_of(SyntaxKind::ClassName)?;
                    Some((
                        "new".to_string(),
                        Receiver::Class(class.text(source).to_string()),
                    ))
                }
                // `foo(a, b)` is `a.foo(b)` — the receiver is the first
                // argument, whose type is unknown.
                SyntaxKind::NameRef => {
                    let name = head.token_of(SyntaxKind::Ident)?;
                    Some((name.text(source).to_string(), Receiver::Unknown))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The role a token plays, as far as the server can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Point {
    /// A class name in any position.
    ClassName(String),
    /// A selector being sent to a receiver: the `bar` of `foo.bar`.
    Selector { name: String, receiver: Receiver },
    /// The name in a method definition: the `bar` of `bar { ... }`.
    MethodName { name: String, owner: Option<String> },
    /// A name bound in an enclosing scope — an argument, a `var`, an instance
    /// variable — either where it is used or where it is declared.
    ///
    /// Resolving it needs the tree rather than the index, so this carries only
    /// the name and the caller looks it up against the scope at the point.
    Local { name: String },
    /// Nothing the server has anything to say about.
    Nothing,
}

/// Classify the token at a point.
pub fn point_at(root: &SyntaxNode, source: &str, offset: u32, bias: Bias) -> Point {
    let Some(path) = token_at(root, offset, bias) else {
        return Point::Nothing;
    };
    classify(&path, source)
}

fn classify(path: &TokenPath<'_>, source: &str) -> Point {
    let token = path.token;
    let text = token.text(source).to_string();

    match token.kind {
        SyntaxKind::ClassName => Point::ClassName(text),
        SyntaxKind::Ident => {
            let Some(parent) = path.parent() else {
                return Point::Nothing;
            };
            match parent.kind {
                // `foo.bar` — the selector is the ident that follows the dot.
                SyntaxKind::MethodCall => {
                    let dot = parent
                        .child_tokens()
                        .filter(|t| t.kind == SyntaxKind::Dot && t.end <= token.start)
                        .last();
                    match dot {
                        Some(dot) => Point::Selector {
                            name: text,
                            receiver: receiver_of(parent, dot.start, source),
                        },
                        None => Point::Nothing,
                    }
                }
                // `bar { ... }` inside a class body.
                SyntaxKind::MethodDef => Point::MethodName {
                    name: text,
                    owner: enclosing_class(&path.ancestors, source),
                },
                // A bare name used as a value, or the name being declared.
                // Both want the same answer: where this binding comes from.
                SyntaxKind::NameRef
                | SyntaxKind::VarDef
                | SyntaxKind::SlotDef
                | SyntaxKind::RestArg => Point::Local { name: text },
                _ => Point::Nothing,
            }
        }
        _ => Point::Nothing,
    }
}

/// The class whose body contains `offset`, defined or extended.
pub fn enclosing_class_at(root: &SyntaxNode, source: &str, offset: u32) -> Option<String> {
    enclosing_class(&ancestors_at(root, offset), source)
}

/// The same, from a path that has already been walked.
///
/// What a method definition belongs to, and equally what a class slot written
/// anywhere above is in scope for.
pub fn enclosing_class(ancestors: &[&SyntaxNode], source: &str) -> Option<String> {
    ancestors
        .iter()
        .rev()
        .find(|n| matches!(n.kind, SyntaxKind::ClassDef | SyntaxKind::ClassExtension))
        .and_then(|n| n.token_of(SyntaxKind::ClassName))
        .map(|t| t.text(source).to_string())
}

/// Resolve a selector to the methods it could mean.
///
/// With a class receiver this is exact: walk the superclass chain and take the
/// first definition, as dispatch would. Otherwise it is every implementor,
/// which is what "no type inference" honestly buys.
pub fn resolve_selector<'a>(
    index: &'a SymbolIndex,
    name: &str,
    receiver: &Receiver,
) -> Vec<&'a sclang_index::Method> {
    match receiver {
        Receiver::Class(class) => {
            // `Foo.bar` is a class-side send. If nothing matches there, fall
            // back to instance-side: the receiver may be a variable that
            // merely looks like a class name, and a wrong-side answer beats
            // none.
            for kind in [MethodKind::Class, MethodKind::Instance] {
                if let Some(m) = index
                    .superclass_chain(class)
                    .into_iter()
                    .find_map(|c| index.method(&c.name, name, kind))
                {
                    return vec![m];
                }
            }
            Vec::new()
        }
        // An instance answers instance-side methods, walking up its
        // superclasses exactly as dispatch would. No class-side fallback:
        // unlike a bare `Foo`, there is no chance this is a variable that
        // merely looks like a class name.
        Receiver::Instance(class) => index
            .superclass_chain(class)
            .into_iter()
            .find_map(|c| index.method(&c.name, name, MethodKind::Instance))
            .into_iter()
            .collect(),

        Receiver::Unknown => index.implementors(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sclang_syntax::parse;

    fn point(source: &str, offset: u32) -> Point {
        let parse = parse(source);
        point_at(&parse.root, source, offset, Bias::Inside)
    }

    #[test]
    fn finds_class_name() {
        assert_eq!(point("SinOsc.ar", 2), Point::ClassName("SinOsc".into()));
    }

    #[test]
    fn finds_selector_with_class_receiver() {
        assert_eq!(
            point("SinOsc.ar", 7),
            Point::Selector {
                name: "ar".into(),
                receiver: Receiver::Class("SinOsc".into()),
            }
        );
    }

    #[test]
    fn finds_selector_with_unknown_receiver() {
        assert_eq!(
            point("x.foo", 2),
            Point::Selector {
                name: "foo".into(),
                receiver: Receiver::Unknown,
            }
        );
    }

    #[test]
    fn chained_call_receiver_is_not_the_first_name() {
        // The receiver of `.c` is the result of `a.b`, which is unknown —
        // not the `a` that happens to start the expression.
        assert_eq!(
            point("Foo.b.c", 6),
            Point::Selector {
                name: "c".into(),
                receiver: Receiver::Unknown,
            }
        );
    }

    #[test]
    fn finds_method_definition_name() {
        assert_eq!(
            point("Foo : Bar { baz { ^1 } }", 13),
            Point::MethodName {
                name: "baz".into(),
                owner: Some("Foo".into()),
            }
        );
    }

    #[test]
    fn finds_method_name_in_class_extension() {
        assert_eq!(
            point("+ Foo { baz { ^1 } }", 9),
            Point::MethodName {
                name: "baz".into(),
                owner: Some("Foo".into()),
            }
        );
    }

    #[test]
    fn bias_before_picks_the_typed_token() {
        let source = "SinOsc.ar";
        let parse = parse(source);
        // The cursor sits after the final `r`. Inside finds nothing, because
        // the offset is the end of the file; Before finds the token typed.
        assert_eq!(
            point_at(&parse.root, source, 9, Bias::Before),
            Point::Selector {
                name: "ar".into(),
                receiver: Receiver::Class("SinOsc".into()),
            }
        );
    }

    #[test]
    fn nothing_on_whitespace() {
        assert_eq!(point("Foo  ", 4), Point::Nothing);
    }

    #[test]
    fn finds_a_local_where_it_is_used() {
        assert_eq!(
            point("{ var foo = 1; foo.postln; }", 15),
            Point::Local { name: "foo".into() }
        );
    }

    #[test]
    fn finds_a_local_where_it_is_declared() {
        assert_eq!(
            point("{ var foo = 1; }", 6),
            Point::Local { name: "foo".into() }
        );
    }

    #[test]
    fn finds_an_argument_where_it_is_declared() {
        assert_eq!(
            point("{ |freq = 440| freq }", 3),
            Point::Local {
                name: "freq".into()
            }
        );
    }
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    use sclang_syntax::parse;

    /// The receiver of the last `.` in the source.
    fn receiver(source: &str) -> Receiver {
        let parse = parse(source);
        let dot = source.rfind('.').expect("a dot") as u32;
        let call = parse
            .root
            .descendants()
            .into_iter()
            .rfind(|n| n.kind == SyntaxKind::MethodCall && n.start <= dot && dot < n.end)
            .expect("a method call");
        receiver_of(call, dot, source)
    }

    #[test]
    fn a_class_name_is_a_class_receiver() {
        assert_eq!(receiver("SinOsc.ar"), Receiver::Class("SinOsc".into()));
    }

    #[test]
    fn constructing_a_class_gives_an_instance_of_it() {
        // `Foo(...)` is `Foo.new(...)`, and this is the case that sends
        // `Pbind(...).play` to `Pattern:play` rather than to every `play`.
        assert_eq!(
            receiver("Pbind(\\dur, 1).play"),
            Receiver::Instance("Pbind".into())
        );
        assert_eq!(
            receiver("Pbind.new(1).play"),
            Receiver::Instance("Pbind".into())
        );
    }

    #[test]
    fn only_new_counts_as_construction() {
        // `Foo.bar(...)` could return anything; nothing but `new` carries the
        // convention.
        assert_eq!(receiver("Buffer.alloc(s, 1).play"), Receiver::Unknown);
    }

    #[test]
    fn literals_are_instances_of_the_class_the_grammar_gives_them() {
        assert_eq!(
            receiver("\"hi\".reverse"),
            Receiver::Instance("String".into())
        );
        assert_eq!(receiver("[1, 2].sum"), Receiver::Instance("Array".into()));
        assert_eq!(
            receiver("{ 1 }.value"),
            Receiver::Instance("Function".into())
        );
        assert_eq!(receiver("42.rand"), Receiver::Instance("Integer".into()));
        assert_eq!(receiver("4.5.round"), Receiver::Instance("Float".into()));
        assert_eq!(
            receiver("\\sym.asString"),
            Receiver::Instance("Symbol".into())
        );
        assert_eq!(receiver("$c.ascii"), Receiver::Instance("Char".into()));
        assert_eq!(receiver("(a: 1).keys"), Receiver::Instance("Event".into()));
    }

    #[test]
    fn a_variable_is_still_unknown() {
        // The whole point of the restraint: nothing here infers a type.
        assert_eq!(receiver("~pattern.play"), Receiver::Unknown);
        assert_eq!(receiver("x.play"), Receiver::Unknown);
    }

    #[test]
    fn construction_chains() {
        // The receiver of the second dot is the whole `Pbind(...).play` call,
        // whose class is not knowable — `play` may return anything.
        assert_eq!(receiver("Pbind(1).play.stop"), Receiver::Unknown);
    }
}
