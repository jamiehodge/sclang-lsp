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

fn visit_tokens(node: &SyntaxNode, f: &mut impl FnMut(&Token)) {
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
    /// A literal class name: `SinOsc.ar`. The one case where dispatch is
    /// statically known, so completion and goto can be exact.
    Class(String),
    /// Anything else. SuperCollider is dynamically typed and this server does
    /// no inference, so the honest answer is every class defining the
    /// selector.
    Unknown,
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

    match before {
        Some(Child::Node(n)) if n.kind == SyntaxKind::ClassRef => n
            .token_of(SyntaxKind::ClassName)
            .map(|t| Receiver::Class(t.text(source).to_string()))
            .unwrap_or(Receiver::Unknown),
        _ => Receiver::Unknown,
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
                    owner: owner_of(path, source),
                },
                _ => Point::Nothing,
            }
        }
        _ => Point::Nothing,
    }
}

/// The class a method definition belongs to, walking out to the enclosing
/// `ClassDef` or `+ Foo` extension.
fn owner_of(path: &TokenPath<'_>, source: &str) -> Option<String> {
    path.ancestors
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
}
