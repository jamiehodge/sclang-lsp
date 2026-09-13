//! Parser machinery: a token cursor, an event sink, and the tree builder.
//!
//! The grammar functions in [`crate::grammar`] never touch the tree directly.
//! They emit a flat stream of [`Event`]s, and [`build_tree`] turns those plus
//! the raw token list — trivia included — into the final CST. Keeping the two
//! apart is what lets the grammar ignore whitespace entirely while the tree
//! stays lossless.

use crate::kind::SyntaxKind;
use crate::lexer::Token;
use crate::tree::{Child, Parse, SyntaxError, SyntaxNode};

/// A flat description of tree structure, produced while parsing.
#[derive(Debug, Clone)]
pub enum Event {
    /// Open a node. `kind` is [`SyntaxKind::ErrorNode`] as a placeholder until
    /// the matching [`Marker`] is completed.
    Start {
        kind: SyntaxKind,
        forward_parent: Option<usize>,
    },
    /// Close the innermost open node.
    Finish,
    /// Attach the next non-trivia token to the innermost open node.
    Token,
    /// A node that was opened then abandoned; ignored by the builder.
    Tombstone,
}

/// A position in the event stream that can later be turned into a node.
///
/// Markers make left-associative parsing natural: parse the left operand,
/// and if an operator follows, wrap what was already parsed by calling
/// [`Marker::precede`].
pub struct Marker {
    pos: usize,
    completed: bool,
}

impl Marker {
    /// Close this node with the given kind.
    pub fn complete(mut self, p: &mut Parser, kind: SyntaxKind) -> CompletedMarker {
        self.completed = true;
        if let Event::Start { kind: slot, .. } = &mut p.events[self.pos] {
            *slot = kind;
        }
        p.events.push(Event::Finish);
        CompletedMarker {
            pos: self.pos,
            kind,
        }
    }

    /// Discard this node. Any tokens consumed since it was opened stay where
    /// they are, attached to the enclosing node.
    pub fn abandon(mut self, p: &mut Parser) {
        self.completed = true;
        if self.pos == p.events.len() - 1 {
            p.events.pop();
        } else {
            p.events[self.pos] = Event::Tombstone;
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        // A marker that is neither completed nor abandoned is a bug in the
        // grammar, and one that would silently corrupt the tree.
        debug_assert!(
            self.completed,
            "Marker dropped without complete() or abandon()"
        );
    }
}

/// A node that has been closed, and can still be wrapped by a new parent.
#[derive(Clone, Copy)]
pub struct CompletedMarker {
    pos: usize,
    kind: SyntaxKind,
}

impl CompletedMarker {
    /// What was built. The grammar needs this to decide whether an expression
    /// may be followed by an argument list, which `lang11d` allows only for a
    /// `name` or a `classname`.
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// Open a new node that starts where this one starts, making this node its
    /// first child. This is how `a + b` becomes `BinaryExpr(a, +, b)` after
    /// `a` has already been parsed.
    pub fn precede(self, p: &mut Parser) -> Marker {
        let new_pos = p.start();
        if let Event::Start { forward_parent, .. } = &mut p.events[self.pos] {
            *forward_parent = Some(new_pos.pos - self.pos);
        }
        new_pos
    }
}

/// The parser: a cursor over non-trivia tokens, plus accumulated events.
pub struct Parser<'a> {
    source: &'a str,
    /// Non-trivia tokens only. Indices here are *not* indices into the full
    /// token list; the builder re-interleaves trivia.
    tokens: Vec<Token>,
    pos: usize,
    events: Vec<Event>,
    errors: Vec<SyntaxError>,
    /// Guards against a grammar bug spinning forever without consuming input.
    fuel: u32,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, all_tokens: &[Token]) -> Self {
        Parser {
            source,
            tokens: all_tokens
                .iter()
                .copied()
                .filter(|t| !t.kind.is_trivia())
                .collect(),
            pos: 0,
            events: Vec::new(),
            errors: Vec::new(),
            fuel: 256,
        }
    }

    // ---- inspection ---------------------------------------------------

    /// The kind at the cursor, or [`SyntaxKind::Eof`].
    pub fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// The kind `n` tokens ahead.
    pub fn nth(&self, n: usize) -> SyntaxKind {
        self.tokens
            .get(self.pos + n)
            .map_or(SyntaxKind::Eof, |t| t.kind)
    }

    /// Text of the token at the cursor.
    pub fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    pub fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        kinds.contains(&self.current())
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// Byte range of the token at the cursor, or the end of input.
    pub fn current_range(&self) -> (u32, u32) {
        match self.tokens.get(self.pos) {
            Some(t) => (t.start, t.end),
            None => {
                let end = self.source.len() as u32;
                (end, end)
            }
        }
    }

    // ---- consumption --------------------------------------------------

    /// Open a new node at the cursor.
    pub fn start(&mut self) -> Marker {
        let pos = self.events.len();
        self.events.push(Event::Start {
            kind: SyntaxKind::ErrorNode,
            forward_parent: None,
        });
        Marker {
            pos,
            completed: false,
        }
    }

    /// Consume the current token into the tree.
    pub fn bump(&mut self) {
        if self.at_end() {
            return;
        }
        self.pos += 1;
        self.fuel = 256;
        self.events.push(Event::Token);
    }

    /// Consume the current token if it matches.
    pub fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consume the expected token, or record an error and consume nothing.
    pub fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.eat(kind) {
            return true;
        }
        self.error(format!("expected {kind:?}, found {:?}", self.current()));
        false
    }

    // ---- errors and recovery ------------------------------------------

    /// Record an error at the cursor without consuming anything.
    pub fn error(&mut self, message: impl Into<String>) {
        let (start, end) = self.current_range();
        self.errors.push(SyntaxError {
            message: message.into(),
            start,
            end,
        });
    }

    /// Record an error and wrap the offending token in an [`SyntaxKind::ErrorNode`],
    /// so the tree keeps it and parsing can continue.
    pub fn error_and_bump(&mut self, message: impl Into<String>) {
        self.error(message);
        if !self.at_end() {
            let m = self.start();
            self.bump();
            m.complete(self, SyntaxKind::ErrorNode);
        }
    }

    /// Skip tokens until one of `recovery` is at the cursor, wrapping whatever
    /// was skipped in an error node. Brace depth is tracked so that skipping
    /// does not escape the construct being recovered.
    pub fn recover_until(&mut self, recovery: &[SyntaxKind]) {
        if self.at_end() || self.at_any(recovery) {
            return;
        }
        let m = self.start();
        let mut depth = 0i32;
        while !self.at_end() {
            match self.current() {
                SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBracket => depth += 1,
                SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBracket => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            if depth == 0 && self.at_any(recovery) {
                break;
            }
            self.bump();
        }
        m.complete(self, SyntaxKind::ErrorNode);
    }

    /// Detect a grammar function that is looping without consuming input.
    /// Returns false once the budget is exhausted, so callers can break.
    pub fn progressing(&mut self) -> bool {
        if self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        true
    }

    pub fn finish(self) -> (Vec<Event>, Vec<SyntaxError>) {
        (self.events, self.errors)
    }
}

/// Turn events plus the full token list into a lossless tree.
///
/// Trivia is attached to whichever node is open when it is reached, which
/// keeps comments near the construct they document.
pub fn build_tree(
    source: &str,
    tokens: &[Token],
    events: Vec<Event>,
    errors: Vec<SyntaxError>,
) -> Parse {
    // Resolve `forward_parent` chains: a node that was later wrapped must be
    // emitted inside its wrapper.
    let mut events = events;
    let mut reordered: Vec<Event> = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        match events[i] {
            Event::Start {
                forward_parent: Some(_),
                ..
            } => {
                // Walk the chain, collecting parents outermost-last.
                let mut chain = vec![i];
                let mut idx = i;
                while let Event::Start {
                    forward_parent: Some(delta),
                    ..
                } = events[idx]
                {
                    idx += delta;
                    chain.push(idx);
                }
                for &node in chain.iter().rev() {
                    if let Event::Start { kind, .. } = events[node] {
                        reordered.push(Event::Start {
                            kind,
                            forward_parent: None,
                        });
                        events[node] = Event::Tombstone;
                    }
                }
            }
            Event::Start {
                forward_parent: None,
                kind,
            } => {
                reordered.push(Event::Start {
                    kind,
                    forward_parent: None,
                });
                events[i] = Event::Tombstone;
            }
            Event::Finish => reordered.push(Event::Finish),
            Event::Token => reordered.push(Event::Token),
            Event::Tombstone => {}
        }
        i += 1;
    }

    let mut stack: Vec<SyntaxNode> = Vec::new();
    let mut token_iter = tokens.iter().peekable();
    let mut root: Option<SyntaxNode> = None;

    /// Move pending trivia into the innermost open node.
    fn flush_trivia<'t>(
        stack: &mut [SyntaxNode],
        iter: &mut std::iter::Peekable<std::slice::Iter<'t, Token>>,
    ) {
        while let Some(t) = iter.peek() {
            if !t.kind.is_trivia() {
                break;
            }
            if let Some(top) = stack.last_mut() {
                top.children.push(Child::Token(**t));
            }
            iter.next();
        }
    }

    for event in reordered {
        match event {
            Event::Start { kind, .. } => {
                // Trivia before a node belongs to the enclosing node, not the
                // new one — so flush first. At the root there is no enclosing
                // node yet, so push first and let the leading trivia (a file's
                // licence header, say) land inside it.
                if stack.is_empty() {
                    stack.push(SyntaxNode {
                        kind,
                        start: 0,
                        end: 0,
                        children: Vec::new(),
                    });
                    flush_trivia(&mut stack, &mut token_iter);
                } else {
                    flush_trivia(&mut stack, &mut token_iter);
                    stack.push(SyntaxNode {
                        kind,
                        start: 0,
                        end: 0,
                        children: Vec::new(),
                    });
                }
            }
            Event::Token => {
                flush_trivia(&mut stack, &mut token_iter);
                if let Some(t) = token_iter.next() {
                    if let Some(top) = stack.last_mut() {
                        top.children.push(Child::Token(*t));
                    }
                }
            }
            Event::Finish => {
                if let Some(mut node) = stack.pop() {
                    set_range(&mut node);
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(Child::Node(node));
                    } else {
                        root = Some(node);
                    }
                }
            }
            Event::Tombstone => {}
        }
    }

    // Anything left (trailing trivia, or tokens after a truncated parse) goes
    // into the root so the tree still covers the whole file.
    let mut root = root.unwrap_or(SyntaxNode {
        kind: SyntaxKind::SourceFile,
        start: 0,
        end: 0,
        children: Vec::new(),
    });
    for t in token_iter {
        root.children.push(Child::Token(*t));
    }
    set_range(&mut root);
    if root.children.is_empty() {
        root.end = source.len() as u32;
    }

    Parse { root, errors }
}

/// A node spans from its first child to its last.
fn set_range(node: &mut SyntaxNode) {
    match (node.children.first(), node.children.last()) {
        (Some(first), Some(last)) => {
            node.start = first.range().0;
            node.end = last.range().1;
        }
        _ => {
            node.start = 0;
            node.end = 0;
        }
    }
}
