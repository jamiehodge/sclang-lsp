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

/// How sure a receiver's class is.
///
/// The distinction is not academic. It decides whether an answer may be
/// *rendered as though it were in the source* — which an inlay hint and a
/// keyword-argument completion both do, and a completion list does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certainty {
    /// The grammar decides it. A string literal is a `String`, `[1, 2]` is an
    /// `Array`, `{ }` is a `Function`, and `this` is the class whose body the
    /// send is written in. Nothing can make these wrong.
    Certain,
    /// Convention decides it. `Foo(...)` is `Foo.new(...)` — that desugaring
    /// is the language's, not a guess — and `*new` returns an instance of the
    /// class it was sent to, almost always via `super.new` or
    /// `super.newCopyArgs`. A class is free to return something else, and a
    /// few do, so this is overwhelmingly right rather than guaranteed.
    Conventional,
}

/// What a `.` is being sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// A class object: a literal class name as in `SinOsc.ar`, or `this` and
    /// `super` inside a class method. A class-side send.
    Class(String),
    /// An instance of a known class: `Pbind(...).play`, `"foo".reverse`, or
    /// `this` inside an instance method.
    ///
    /// Not inference — see [`instance_class`] and [`SelfContext`] for what
    /// qualifies, and `certainty` for how sure each case is.
    Instance { class: String, certainty: Certainty },
    /// Anything else. SuperCollider is dynamically typed and this server does
    /// no inference, so the honest answer is every class defining the
    /// selector.
    Unknown,
}

impl Receiver {
    /// The class named, whichever side the receiver is on.
    pub fn class(&self) -> Option<&str> {
        match self {
            Receiver::Class(class) => Some(class),
            Receiver::Instance { class, .. } => Some(class),
            Receiver::Unknown => None,
        }
    }

    /// Whether the class is a fact of the grammar rather than a convention.
    ///
    /// A class name written in the source is always one. `Unknown` never is.
    pub fn is_certain(&self) -> bool {
        match self {
            Receiver::Class(_) => true,
            Receiver::Instance { certainty, .. } => *certainty == Certainty::Certain,
            Receiver::Unknown => false,
        }
    }
}

/// What `this` and `super` mean at a point: the class body they are written
/// in, and which side of it.
///
/// `this` is the one receiver a dynamically typed language hands you for
/// nothing. It is 8,419 of the 37,707 sends in the stock class library — a
/// little under a quarter — and the class it refers to is written at the top
/// of the very node the cursor is inside. Reading it off the tree is not
/// inference; it is the same act as reading `String` off a string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfContext {
    /// The class being defined or extended.
    pub owner: String,
    /// Where `super` starts its lookup: the `: Super` written on the class, or
    /// `Object` for a class written without one, matching what `sclang-index`
    /// records. `None` inside a `+ Foo { }` extension, which names no
    /// superclass — three sends in the stock library, and saying nothing beats
    /// guessing.
    pub superclass: Option<String>,
    /// Class side inside `*foo { }`, instance side otherwise. A class method's
    /// `this` is the class object itself, not an instance of it.
    pub side: MethodKind,
}

/// What resolving a receiver may need beyond the node itself.
///
/// Two things: what `this` means here, and the tree, so a bare name can be
/// traced to the declaration that bound it.
pub struct ReceiverContext<'a> {
    pub root: &'a SyntaxNode,
    pub self_ctx: Option<SelfContext>,
}

impl<'a> ReceiverContext<'a> {
    pub fn at(root: &'a SyntaxNode, source: &str, offset: u32) -> Self {
        ReceiverContext {
            root,
            self_ctx: self_context_at(root, source, offset),
        }
    }

    /// The same, from a path that has already been walked.
    pub fn from_path(root: &'a SyntaxNode, ancestors: &[&SyntaxNode], source: &str) -> Self {
        ReceiverContext {
            root,
            self_ctx: self_context(ancestors, source),
        }
    }
}

/// The class body and side containing `offset`.
pub fn self_context_at(root: &SyntaxNode, source: &str, offset: u32) -> Option<SelfContext> {
    self_context(&ancestors_at(root, offset), source)
}

/// The class whose body contains `offset`, defined or extended.
///
/// Weaker than [`self_context_at`], which also wants a method around the
/// point. A class slot is visible anywhere in the class body, so looking one
/// up only needs this much.
pub fn enclosing_class_at(root: &SyntaxNode, source: &str, offset: u32) -> Option<String> {
    enclosing_class(&ancestors_at(root, offset), source)
}

/// The same, from a path that has already been walked.
pub fn enclosing_class(ancestors: &[&SyntaxNode], source: &str) -> Option<String> {
    ancestors
        .iter()
        .rev()
        .find(|n| matches!(n.kind, SyntaxKind::ClassDef | SyntaxKind::ClassExtension))
        .and_then(|n| n.token_of(SyntaxKind::ClassName))
        .map(|t| t.text(source).to_string())
}

/// The same, from a path that has already been walked.
///
/// A function body inside a method does not change what `this` means —
/// closures capture it — so this looks through every enclosing block for the
/// method and the class, rather than stopping at the innermost scope.
pub fn self_context(ancestors: &[&SyntaxNode], source: &str) -> Option<SelfContext> {
    let class = ancestors
        .iter()
        .rev()
        .find(|n| matches!(n.kind, SyntaxKind::ClassDef | SyntaxKind::ClassExtension))?;
    let owner = enclosing_class(ancestors, source)?;

    let superclass = class
        .child_of(SyntaxKind::SuperClass)
        .and_then(|s| s.token_of(SyntaxKind::ClassName))
        .map(|t| t.text(source).to_string())
        .or_else(|| {
            (class.kind == SyntaxKind::ClassDef && owner != "Object").then(|| "Object".to_string())
        });

    // Outside a method body there is no receiver for `this` to be, so there is
    // nothing to say.
    let method = ancestors
        .iter()
        .rev()
        .find(|n| n.kind == SyntaxKind::MethodDef)?;
    let side = if sclang_index::is_class_method(method) {
        MethodKind::Class
    } else {
        MethodKind::Instance
    };

    Some(SelfContext {
        owner,
        superclass,
        side,
    })
}

/// The class an expression is known to produce, where that is knowable without
/// inferring anything, and how sure the answer is.
///
/// The two tiers are [`Certainty`]'s, and keeping them apart is the whole
/// point: a literal's class is a fact of the grammar and may be stated as one,
/// while `Foo.new` returning a `Foo` is a convention good enough to navigate
/// by and not good enough to print into the buffer.
pub fn instance_class(node: &SyntaxNode, source: &str) -> Option<(String, Certainty)> {
    use Certainty::{Certain, Conventional};

    match node.kind {
        // `Foo(...)`, which the parser leaves as a call on a class reference.
        SyntaxKind::CallExpr => Some((class_named(node, source)?, Conventional)),

        // `Foo.new(...)`, spelled out. Any other selector is unknown: only
        // `new` has the convention behind it.
        SyntaxKind::MethodCall => {
            let selector = node.token_of(SyntaxKind::Ident)?;
            if selector.text(source) == "new" {
                Some((class_named(node, source)?, Conventional))
            } else {
                None
            }
        }

        SyntaxKind::FunctionBlock => Some(("Function".to_string(), Certain)),
        SyntaxKind::EventLiteral => Some(("Event".to_string(), Certain)),

        // `Set[...]` names its own class; a bare `[...]` is an Array.
        SyntaxKind::Collection => Some((
            class_named(node, source).unwrap_or_else(|| "Array".to_string()),
            Certain,
        )),
        SyntaxKind::LiteralList => Some(("Array".to_string(), Certain)),

        SyntaxKind::Literal => {
            // `floatp : floatr pie | integer pie | pie` — a `pi` suffix makes
            // the whole literal a Float whatever it is written on. The lexer
            // gives `2pi` two tokens, so reading only the first called it an
            // Integer, and called it that with `Certain` behind it: an inlay
            // hint would have written Integer's parameter names into a send to
            // a Float.
            if node.child_tokens().any(|t| t.kind == SyntaxKind::PiKw) {
                return Some(("Float".to_string(), Certain));
            }

            let token = node.child_tokens().find(|t| t.kind.is_literal())?;
            Some((
                match token.kind {
                    SyntaxKind::String => "String",
                    SyntaxKind::Integer | SyntaxKind::RadixInteger | SyntaxKind::HexInteger => {
                        "Integer"
                    }
                    SyntaxKind::Float => "Float",
                    SyntaxKind::Symbol => "Symbol",
                    SyntaxKind::Char => "Char",
                    // `true`, `false` and `nil` are each the sole instance of
                    // a class, which is as much a fact of the grammar as a
                    // string literal being a String.
                    SyntaxKind::TrueKw => "True",
                    SyntaxKind::FalseKw => "False",
                    SyntaxKind::NilKw => "Nil",
                    // An accidental like `4s` is a degree, not a plain number,
                    // and pinning it down is not worth being wrong about.
                    _ => return None,
                }
                .to_string(),
                Certain,
            ))
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
fn receiver_of(
    call: &SyntaxNode,
    dot_start: u32,
    source: &str,
    ctx: &ReceiverContext<'_>,
) -> Receiver {
    // The receiver is the last node before the dot. Taking it positionally
    // rather than by index keeps this correct for `a.b.c`, where the receiver
    // of the second dot is the whole `a.b` call.
    let before = call
        .children
        .iter()
        .rev()
        .find(|c| c.range().1 <= dot_start);

    receiver_from(before, source, ctx)
}

/// Turn the node left of a dot into a receiver.
pub(crate) fn receiver_from(
    node: Option<&Child>,
    source: &str,
    ctx: &ReceiverContext<'_>,
) -> Receiver {
    let Some(Child::Node(n)) = node else {
        return Receiver::Unknown;
    };

    if n.kind == SyntaxKind::ClassRef {
        return n
            .token_of(SyntaxKind::ClassName)
            .map(|t| Receiver::Class(t.text(source).to_string()))
            .unwrap_or(Receiver::Unknown);
    }

    if n.kind == SyntaxKind::NameRef {
        let Some(ident) = n.token_of(SyntaxKind::Ident) else {
            return Receiver::Unknown;
        };
        let name = ident.text(source);

        // `this` and `super` name a class the grammar has already settled: the
        // one whose body the send is written in.
        if let Some(self_ctx) = &ctx.self_ctx {
            match name {
                "this" => return self_receiver(&self_ctx.owner, self_ctx.side),
                // `super` starts lookup one class up, and unlike `this` it
                // cannot be a subclass: the class is written down and fixed.
                "super" => {
                    return self_ctx
                        .superclass
                        .as_deref()
                        .map(|sup| self_receiver(sup, self_ctx.side))
                        .unwrap_or(Receiver::Unknown)
                }
                _ => {}
            }
        }
        // The rest of the pseudo-variables live in a running image.
        if crate::scope::PSEUDO_VARIABLES.contains(&name) {
            return Receiver::Unknown;
        }

        return local_class(name, source, ctx, n.start)
            .map(|(class, certainty)| Receiver::Instance { class, certainty })
            .unwrap_or(Receiver::Unknown);
    }

    instance_class(n, source)
        .map(|(class, certainty)| Receiver::Instance { class, certainty })
        .unwrap_or(Receiver::Unknown)
}

/// The class a bare name was initialised with: `var x = Foo.new` leaves a
/// `Foo` in `x`.
///
/// One step weaker than `Foo.new` written inline, and for a different reason
/// than the constructor convention: a variable can be assigned again, and
/// nothing here follows assignments. So it is used exactly where `Foo.new`
/// already is — to navigate and to complete — and never where a wrong answer
/// would print itself into the buffer.
///
/// Arguments are deliberately excluded. `|clock = TempoClock.new|` says what
/// happens when the parameter is *not* passed, and reading it as the class of
/// `clock` would be inference about every caller rather than a fact about the
/// declaration.
fn local_class(
    name: &str,
    source: &str,
    ctx: &ReceiverContext<'_>,
    offset: u32,
) -> Option<(String, Certainty)> {
    let local = crate::scope::locals_at(ctx.root, source, offset)
        .into_iter()
        .find(|l| l.name == name)?;
    if local.kind == crate::scope::LocalKind::Argument || local.default.is_none() {
        return None;
    }

    // The initialiser is a node, so ask it directly rather than re-parsing the
    // text the declaration happened to record. Found by descending to the
    // declaration rather than by collecting every node in the file, which this
    // is called often enough to feel.
    let decl = ancestors_at(ctx.root, local.range.start)
        .into_iter()
        .rfind(|n| n.start == local.range.start && n.end == local.range.end)?;
    let eq = decl.token_of(SyntaxKind::Eq)?;
    let init = decl.child_nodes().find(|n| n.start >= eq.end)?;

    // Conventional whatever the initialiser was: `var x = "a"` is a String
    // only until something assigns to `x`.
    Some((instance_class(init, source)?.0, Certainty::Conventional))
}

/// `this` in a class method is the class object; in an instance method it is
/// an instance of the class. Either way the class itself is a fact of the
/// grammar.
fn self_receiver(class: &str, side: MethodKind) -> Receiver {
    match side {
        MethodKind::Class => Receiver::Class(class.to_string()),
        MethodKind::Instance => Receiver::Instance {
            class: class.to_string(),
            certainty: Certainty::Certain,
        },
    }
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
    let ctx = ReceiverContext::from_path(root, &path, source);
    let (selector, receiver) = callee(call, arg_list, source, &ctx)?;
    Some(CallSite {
        arg_list,
        selector,
        receiver,
        offset,
    })
}

/// The selector a call invokes, and what it is sent to.
///
/// `ctx` is what a receiver needs beyond its own node — a caller walking
/// descendants can build one with [`ReceiverContext::at`].
pub fn callee(
    call: &SyntaxNode,
    arg_list: &SyntaxNode,
    source: &str,
    ctx: &ReceiverContext<'_>,
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
                receiver_of(call, dot.start, source, ctx),
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
    let ctx = ReceiverContext::from_path(path.ancestors[0], &path.ancestors, source);

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
                            receiver: receiver_of(parent, dot.start, source, &ctx),
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

/// What a selector resolves to, and how tightly.
pub struct Resolution<'a> {
    pub methods: Vec<&'a sclang_index::Method>,
    /// True when the receiver's class was known and its chain answered, so the
    /// single method here is the one dispatch would reach. False when this is
    /// the honest list of every class defining the selector.
    pub exact: bool,
}

impl<'a> Resolution<'a> {
    /// The one method dispatch reaches, when there is exactly one.
    pub fn single(&self) -> Option<&'a sclang_index::Method> {
        self.exact.then(|| self.methods.first().copied()).flatten()
    }
}

/// Resolve a selector to the methods it could mean.
///
/// With a known receiver this walks the superclass chain and takes the first
/// definition, as dispatch would. Where that finds nothing the answer is every
/// implementor — the same answer an unknown receiver gets.
///
/// That fallback matters most for `this`. A superclass's `this` is an instance
/// of whichever subclass was actually called, so a chain miss does not mean
/// the selector does not exist: `SequenceableCollection`'s `choose` sends
/// `this.at`, which only its subclasses define. 1.6% of `this.` sends in the
/// stock class library land there. Narrowing must never turn an honest list
/// into nothing.
pub fn resolve_selector<'a>(
    index: &'a SymbolIndex,
    name: &str,
    receiver: &Receiver,
) -> Resolution<'a> {
    let narrowed = match receiver {
        Receiver::Class(class) => class_side(index, class, name),
        // An instance answers instance-side methods, walking up its
        // superclasses exactly as dispatch would.
        Receiver::Instance { class, .. } => index
            .superclass_chain(class)
            .into_iter()
            .find_map(|c| index.method(&c.name, name, MethodKind::Instance)),
        Receiver::Unknown => None,
    };

    match narrowed {
        Some(method) => Resolution {
            methods: vec![method],
            exact: true,
        },
        None => Resolution {
            methods: index.implementors(name),
            exact: false,
        },
    }
}

/// Dispatch for a send to a class object.
///
/// A class object answers its own class-side methods first. Failing that it is
/// still an object — an instance of `Class`, which is where `name`,
/// `allSubclasses` and `dumpInterface` live, and this is what makes
/// `UnitTest *runAll`'s `this.allSubclasses` resolve. Failing that too, the
/// class's own instance side, because a `Foo.bar` naming an instance method is
/// a common enough thing to write that a wrong-side answer beats none.
fn class_side<'a>(
    index: &'a SymbolIndex,
    class: &str,
    name: &str,
) -> Option<&'a sclang_index::Method> {
    let chain = |from: &str, kind| {
        index
            .superclass_chain(from)
            .into_iter()
            .find_map(|c| index.method(&c.name, name, kind))
    };
    chain(class, MethodKind::Class)
        .or_else(|| chain("Class", MethodKind::Instance))
        .or_else(|| chain(class, MethodKind::Instance))
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
        let ctx = ReceiverContext::at(&parse.root, source, dot);
        receiver_of(call, dot, source, &ctx)
    }

    /// An instance whose class the grammar settles.
    fn certain(class: &str) -> Receiver {
        Receiver::Instance {
            class: class.into(),
            certainty: Certainty::Certain,
        }
    }

    /// An instance whose class a convention settles.
    fn conventional(class: &str) -> Receiver {
        Receiver::Instance {
            class: class.into(),
            certainty: Certainty::Conventional,
        }
    }

    #[test]
    fn a_class_name_is_a_class_receiver() {
        assert_eq!(receiver("SinOsc.ar"), Receiver::Class("SinOsc".into()));
    }

    #[test]
    fn constructing_a_class_gives_an_instance_of_it() {
        // `Foo(...)` is `Foo.new(...)`, and this is the case that sends
        // `Pbind(...).play` to `Pattern:play` rather than to every `play`.
        assert_eq!(receiver("Pbind(\\dur, 1).play"), conventional("Pbind"));
        assert_eq!(receiver("Pbind.new(1).play"), conventional("Pbind"));
    }

    #[test]
    fn only_new_counts_as_construction() {
        // `Foo.bar(...)` could return anything; nothing but `new` carries the
        // convention.
        assert_eq!(receiver("Buffer.alloc(s, 1).play"), Receiver::Unknown);
    }

    #[test]
    fn literals_are_instances_of_the_class_the_grammar_gives_them() {
        assert_eq!(receiver("\"hi\".reverse"), certain("String"));
        assert_eq!(receiver("[1, 2].sum"), certain("Array"));
        assert_eq!(receiver("{ 1 }.value"), certain("Function"));
        assert_eq!(receiver("42.rand"), certain("Integer"));
        assert_eq!(receiver("4.5.round"), certain("Float"));
        assert_eq!(receiver("\\sym.asString"), certain("Symbol"));
        assert_eq!(receiver("$c.ascii"), certain("Char"));
        assert_eq!(receiver("(a: 1).keys"), certain("Event"));
        // Each of these is the sole instance of its class, which is as much a
        // fact of the grammar as a string literal being a String.
        assert_eq!(receiver("true.if"), certain("True"));
        assert_eq!(receiver("false.not"), certain("False"));
        assert_eq!(receiver("nil.isNil"), certain("Nil"));
    }

    #[test]
    fn a_pi_suffix_makes_the_whole_literal_a_float() {
        // `2pi` is 6.28…, and the lexer gives it as two tokens. Reading only
        // the first called it an Integer — and called it that with `Certain`
        // behind it, so an inlay hint would have written Integer's parameter
        // names into a send to a Float.
        assert_eq!(receiver("2pi.round"), certain("Float"));
        assert_eq!(receiver("0.5pi.round"), certain("Float"));
        assert_eq!(receiver("pi.round"), certain("Float"));
        // Without the suffix it is still an integer.
        assert_eq!(receiver("2.round"), certain("Integer"));
    }

    #[test]
    fn an_accidental_is_still_left_alone() {
        // `4s` is a degree rather than a plain number, and pinning it down is
        // not worth being wrong about.
        assert_eq!(receiver("4s.value"), Receiver::Unknown);
    }

    #[test]
    fn a_variable_is_still_unknown() {
        // The whole point of the restraint: nothing here infers a type.
        assert_eq!(receiver("~pattern.play"), Receiver::Unknown);
        assert_eq!(receiver("x.play"), Receiver::Unknown);
    }

    #[test]
    fn a_variable_takes_the_class_it_was_initialised_with() {
        assert_eq!(
            receiver("( var x = Pbind.new; x.play )"),
            conventional("Pbind")
        );
        assert_eq!(
            receiver("( var x = Pbind(1); x.play )"),
            conventional("Pbind")
        );
        assert_eq!(
            receiver("( var x = \"hi\"; x.reverse )"),
            conventional("String")
        );
    }

    #[test]
    fn an_instance_variable_counts_the_same_way() {
        assert_eq!(
            receiver("Foo : Bar { var x = Pbind.new; m { ^x.play } }"),
            conventional("Pbind")
        );
    }

    #[test]
    fn an_uninitialised_variable_says_nothing() {
        assert_eq!(receiver("( var x; x.play )"), Receiver::Unknown);
    }

    #[test]
    fn an_argument_default_is_not_the_arguments_class() {
        // `|clock = TempoClock.new|` says what happens when the parameter is
        // not passed. Every caller is free to pass anything at all, so reading
        // it as the class of `clock` would be inference about them.
        assert_eq!(
            receiver("Foo : Bar { m { |clock = TempoClock.new| ^clock.beats } }"),
            Receiver::Unknown
        );
    }

    #[test]
    fn initialisers_do_not_chain() {
        // `y` is a bare name, not a construction, and following it would be
        // the first step into inference proper.
        assert_eq!(
            receiver("( var y = Pbind.new, x = y; x.play )"),
            Receiver::Unknown
        );
    }

    #[test]
    fn an_inner_binding_shadows_when_resolving_a_receiver() {
        let source = "( var x = Pbind.new; { |x| x.play } )";
        assert_eq!(receiver(source), Receiver::Unknown);
    }

    #[test]
    fn construction_chains() {
        // The receiver of the second dot is the whole `Pbind(...).play` call,
        // whose class is not knowable — `play` may return anything.
        assert_eq!(receiver("Pbind(1).play.stop"), Receiver::Unknown);
    }

    #[test]
    fn this_is_an_instance_of_the_class_being_defined() {
        assert_eq!(receiver("Foo : Bar { m { ^this.baz } }"), certain("Foo"));
    }

    #[test]
    fn this_in_a_class_method_is_the_class_object() {
        // `^this.multiNew(freq)` in a `*ar` is a class-side send, which is why
        // this is a different receiver rather than a different certainty.
        assert_eq!(
            receiver("SinOsc : UGen { *ar { ^this.multiNew } }"),
            Receiver::Class("SinOsc".into())
        );
    }

    #[test]
    fn super_starts_one_class_up() {
        assert_eq!(receiver("Foo : Bar { m { ^super.baz } }"), certain("Bar"));
        assert_eq!(
            receiver("Foo : Bar { *new { ^super.new } }"),
            Receiver::Class("Bar".into())
        );
    }

    #[test]
    fn a_class_written_without_a_superclass_still_has_one() {
        // sclang resolves it to Object, and so does the index.
        assert_eq!(receiver("Foo { m { ^super.baz } }"), certain("Object"));
    }

    #[test]
    fn super_in_an_extension_names_no_superclass() {
        // `+ Foo { }` does not write one down, and guessing would be worse
        // than saying nothing. `this` is unaffected.
        assert_eq!(receiver("+ Foo { m { ^super.baz } }"), Receiver::Unknown);
        assert_eq!(receiver("+ Foo { m { ^this.baz } }"), certain("Foo"));
    }

    #[test]
    fn a_closure_does_not_change_what_this_means() {
        // Functions capture `this`, so a send inside one is still the method's
        // receiver.
        assert_eq!(
            receiver("Foo : Bar { m { ^{ this.baz } } }"),
            certain("Foo")
        );
    }

    #[test]
    fn this_outside_a_method_says_nothing() {
        // At the top of a script there is no receiver for it to be.
        assert_eq!(receiver("this.baz"), Receiver::Unknown);
    }

    #[test]
    fn other_pseudo_variables_are_not_guessed_at() {
        // `thisProcess` and friends live in a running image.
        assert_eq!(
            receiver("Foo : Bar { m { ^thisProcess.baz } }"),
            Receiver::Unknown
        );
    }
}
