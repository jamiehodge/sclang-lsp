//! Indentation, computed from the tree.
//!
//! This is an *indenter*, not a formatter: it rewrites the leading whitespace
//! of a line and nothing else. Every line break the author wrote survives, and
//! so does every column they aligned by hand — which matters more in
//! SuperCollider than in most languages, because its two central idioms are
//! written as aligned columns:
//!
//! ```text
//! Pbind(
//!     \degree, Pseq([0, 2, 4], inf),
//!     \dur,    0.25
//! )
//! ```
//!
//! A reflowing formatter either collapses that onto one line or explodes it to
//! one argument per line. Neither is what anyone wants, and there is no agreed
//! SuperCollider style to converge on, so choosing line breaks would mean
//! minting one. Choosing indentation does not.
//!
//! What the tree buys over counting brackets is two things. The obvious one is
//! that `$(`, `'('`, `"("` and `// (` are simply not `LParen` tokens, so the
//! four cases that defeat a line-local regex never arise. The subtler one is
//! [`LineIndent::Follows`]: a line continuing an expression that began earlier
//! is recognised as such, and keeps whatever alignment it was given.
//!
//! Every answer here is *relative to an earlier line* rather than an absolute
//! depth. That is what lets the two kinds compose: a call split across lines
//! inside a hand-aligned continuation indents from where the continuation
//! actually sits, instead of from where its nesting says it ought to.

use crate::kind::SyntaxKind;
use crate::lexer::Token;
use crate::tree::{Child, Parse, SyntaxNode};

/// What should happen to one line's leading whitespace.
///
/// Every variant that names a line names an *earlier* one, so a single forward
/// pass resolves the lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineIndent {
    /// A statement at the outermost level: no indent.
    Root,
    /// A statement inside a bracket that opened on this line: one level
    /// deeper than it.
    Inside(u32),
    /// The bracket closing what opened on this line: exactly its indent, so a
    /// closer lands under whatever opened it.
    Align(u32),
    /// A continuation of the statement that began on this line: its indent
    /// plus the offset this line already had from it, so hand alignment
    /// survives and the line still moves with its block.
    Follows(u32),
    /// Leave alone: blank, or interior to a multi-line string or comment.
    Leave,
}

/// How one indent level is spelled. Both fields come from the editor rather
/// than from us — in LSP they arrive as `FormattingOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentStyle {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

impl Default for IndentStyle {
    fn default() -> Self {
        IndentStyle {
            tab_size: 4,
            insert_spaces: true,
        }
    }
}

impl IndentStyle {
    /// Spell a column count as whitespace.
    ///
    /// With tabs, a remainder that does not fill a whole tab is spelled in
    /// spaces: a [`LineIndent::Follows`] line can sit at any column, and
    /// rounding it to a tab stop would destroy the alignment this module
    /// exists to preserve.
    pub fn render(self, columns: u32) -> String {
        if self.insert_spaces || self.tab_size == 0 {
            return " ".repeat(columns as usize);
        }
        let mut out = "\t".repeat((columns / self.tab_size) as usize);
        out.push_str(&" ".repeat((columns % self.tab_size) as usize));
        out
    }

    /// One level, in columns.
    fn level(self) -> u32 {
        self.tab_size.max(1)
    }
}

/// The leading whitespace of a line: its length in bytes, and its width in
/// columns once tabs are expanded.
pub fn leading_whitespace(line: &str, tab_size: u32) -> (u32, u32) {
    let mut bytes = 0;
    let mut columns = 0;
    for c in line.chars() {
        match c {
            // Advance to the next tab stop, which is not the same as adding
            // `tab_size` unless we are already standing on one.
            '\t' if tab_size > 0 => columns = (columns / tab_size + 1) * tab_size,
            '\t' => {}
            c if c.is_whitespace() => columns += 1,
            _ => break,
        }
        bytes += c.len_utf8() as u32;
    }
    (bytes, columns)
}

/// The target indent of every line, in columns. `None` means leave that line
/// alone.
///
/// This is the entry point a caller wants: [`line_indents`] describes what each
/// line *is*, and this resolves those descriptions against the source and the
/// requested style.
pub fn indent_columns(source: &str, parse: &Parse, style: IndentStyle) -> Vec<Option<u32>> {
    let starts = line_starts(source);
    let indents = indents_with(source, &starts, parse);

    let current = |line: u32| -> u32 {
        let (start, end) = line_bounds(source, &starts, line);
        leading_whitespace(&source[start as usize..end as usize], style.tab_size).1
    };

    // A line that was left alone contributes whatever indent it already had.
    // The only way to reach a line not yet resolved is a self-reference, which
    // the walk cannot produce; falling back keeps that total rather than
    // panicking if it ever does.
    fn resolved(out: &[Option<u32>], line: u32, current: impl Fn(u32) -> u32) -> u32 {
        out.get(line as usize)
            .copied()
            .flatten()
            .unwrap_or_else(|| current(line))
    }

    let mut out: Vec<Option<u32>> = Vec::with_capacity(indents.len());
    for (line, indent) in indents.iter().enumerate() {
        let target = match *indent {
            LineIndent::Root => Some(0),
            LineIndent::Inside(m) => Some(resolved(&out, m, current) + style.level()),
            LineIndent::Align(m) => Some(resolved(&out, m, current)),
            LineIndent::Follows(m) => {
                let offset = current(line as u32).saturating_sub(current(m));
                Some(resolved(&out, m, current) + offset)
            }
            LineIndent::Leave => None,
        };
        out.push(target);
    }
    out
}

/// The source with every line's leading whitespace rewritten.
///
/// The convenience form, for tests, sweeps and anything wanting text rather
/// than edits. A language server wants [`indent_columns`] instead, so that an
/// unchanged line produces no edit at all.
pub fn reindent(source: &str, parse: &Parse, style: IndentStyle) -> String {
    let starts = line_starts(source);
    let targets = indent_columns(source, parse, style);

    let mut out = String::with_capacity(source.len());
    for (line, target) in targets.iter().enumerate() {
        let line = line as u32;
        let (start, end) = line_bounds(source, &starts, line);
        let text = &source[start as usize..end as usize];
        match target {
            // A line of nothing but whitespace keeps none of it: there is
            // nothing there to indent, and what would be left is trailing
            // whitespace.
            Some(columns) => {
                let (bytes, _) = leading_whitespace(text, style.tab_size);
                if (bytes as usize) < text.len() {
                    out.push_str(&style.render(*columns));
                    out.push_str(&text[bytes as usize..]);
                }
            }
            None => out.push_str(text),
        }
        // Put back exactly the terminator the line had, CRLF included.
        out.push_str(&source[end as usize..next_start(&starts, line, source)]);
    }
    out
}

/// What each line is, before any style is applied.
pub fn line_indents(source: &str, parse: &Parse) -> Vec<LineIndent> {
    let starts = line_starts(source);
    indents_with(source, &starts, parse)
}

fn indents_with(source: &str, starts: &[u32], parse: &Parse) -> Vec<LineIndent> {
    let mut walker = Walker {
        source,
        starts,
        out: vec![None; starts.len()],
    };
    walker.walk(&parse.root, None, None);
    walker
        .out
        .into_iter()
        .map(|i| i.unwrap_or(LineIndent::Leave))
        .collect()
}

struct Walker<'a> {
    source: &'a str,
    starts: &'a [u32],
    out: Vec<Option<LineIndent>>,
}

impl Walker<'_> {
    fn line(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(exact) => exact as u32,
            Err(next) => next as u32 - 1,
        }
    }

    /// Record what a line should do, if this token is what begins it.
    ///
    /// Whitespace never counts: the only line it could begin is a blank one,
    /// and a blank line has no indentation to get right. Comments do count, so
    /// a comment on its own line moves with the block it sits in.
    fn record(&mut self, token: &Token, indent: LineIndent) {
        if token.kind == SyntaxKind::Whitespace || !starts_the_line(self.source, token.start) {
            return;
        }
        let line = self.line(token.start) as usize;
        if self.out[line].is_none() {
            self.out[line] = Some(indent);
        }
    }

    /// `body` is the line the enclosing bracket opened on, if there is one.
    /// `origin` is the line an enclosing statement began on, if one did.
    fn walk(&mut self, node: &SyntaxNode, body: Option<u32>, origin: Option<u32>) {
        let span = body_span(node);
        let opener = span.map(|(open, _)| self.line(node.children[open].range().0));

        for (i, child) in node.children.iter().enumerate() {
            let inside = matches!(span, Some((open, close)) if i > open && i < close);
            // Crossing into a body starts a fresh statement context: nothing
            // outside the bracket can make a line inside it a continuation.
            let (child_body, child_origin) = if inside {
                (opener, None)
            } else {
                (body, origin)
            };

            match child {
                Child::Token(token) => {
                    let indent = match span {
                        Some((_, close)) if i == close => LineIndent::Align(opener.unwrap_or(0)),
                        _ => self.statement_or_continuation(token, child_body, child_origin),
                    };
                    self.record(token, indent);
                }
                Child::Node(child_node) => {
                    // A list of statements owns no statement of its own, so it
                    // cannot make its later members continuations of its first.
                    let next_origin = if transparent(child_node.kind) {
                        child_origin
                    } else {
                        child_origin.or(Some(self.line(child_node.start)))
                    };
                    self.walk(child_node, child_body, next_origin);
                }
            }
        }
    }

    fn statement_or_continuation(
        &self,
        token: &Token,
        body: Option<u32>,
        origin: Option<u32>,
    ) -> LineIndent {
        let line = self.line(token.start);
        match origin {
            Some(m) if m < line => LineIndent::Follows(m),
            _ => match body {
                Some(m) => LineIndent::Inside(m),
                None => LineIndent::Root,
            },
        }
    }
}

/// The child indices of a node's opening and closing delimiters, if it has
/// them.
///
/// A node indents its body exactly when it is bracketed. That falls out of the
/// tree's own shape rather than from a list of kinds, which is what makes
/// `SourceFile` and `ExprSeq` transparent for free and keeps `|a, b|` from
/// indenting: `Pipe` is not a bracket, whatever an editor's grammar says.
///
/// An unclosed construct reports a body running to the end of the node, which
/// is the right shape for a buffer someone is still typing into.
fn body_span(node: &SyntaxNode) -> Option<(usize, usize)> {
    let open = node.children.iter().position(|c| is_opener(c.kind()))?;
    let close = node
        .children
        .iter()
        .enumerate()
        .skip(open + 1)
        .rev()
        .find(|(_, c)| is_closer(c.kind()))
        .map(|(i, _)| i)
        .unwrap_or(node.children.len());
    Some((open, close))
}

fn is_opener(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LParen
            | SyntaxKind::LBrace
            | SyntaxKind::LBracket
            // `#{` is one token, and it opens a brace.
            | SyntaxKind::BeginClosedFunc
    )
}

fn is_closer(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::RParen | SyntaxKind::RBrace | SyntaxKind::RBracket
    )
}

/// Nodes holding a sequence of statements rather than being one.
///
/// The same pair `selection_range::only_wraps` names, for the same reason: a
/// second statement in a sequence is not a continuation of the first.
fn transparent(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::SourceFile | SyntaxKind::ExprSeq)
}

/// Whether only whitespace precedes this offset on its line.
fn starts_the_line(source: &str, offset: u32) -> bool {
    source[..offset as usize]
        .rsplit('\n')
        .next()
        .is_some_and(|before| before.chars().all(char::is_whitespace))
}

/// Byte offsets of every line start. Split on `\n` only, matching what the
/// server's `LineIndex` does, so the two agree on what line a thing is on.
fn line_starts(source: &str) -> Vec<u32> {
    let mut out = vec![0];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            out.push(i as u32 + 1);
        }
    }
    out
}

/// Byte range of a line, excluding its terminator.
fn line_bounds(source: &str, starts: &[u32], line: u32) -> (u32, u32) {
    let len = source.len() as u32;
    let start = match starts.get(line as usize) {
        Some(s) => *s,
        None => return (len, len),
    };
    let end = match starts.get(line as usize + 1) {
        Some(next) => {
            let mut e = next - 1;
            if e > start && source.as_bytes().get(e as usize - 1) == Some(&b'\r') {
                e -= 1;
            }
            e
        }
        None => len,
    };
    (start, end)
}

/// Where the next line begins, or the end of the source on the last line.
fn next_start(starts: &[u32], line: u32, source: &str) -> usize {
    starts
        .get(line as usize + 1)
        .map(|s| *s as usize)
        .unwrap_or(source.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, parse_script};

    /// Lines joined into a file, so an expectation reads as the code it is
    /// about.
    fn file(lines: &[&str]) -> String {
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }

    /// A script reindented with four spaces.
    fn script(lines: &[&str]) -> String {
        let source = file(lines);
        reindent(&source, &parse_script(&source), IndentStyle::default())
    }

    /// A `.sc` class file reindented with four spaces.
    fn class_file(lines: &[&str]) -> String {
        let source = file(lines);
        reindent(&source, &parse(&source), IndentStyle::default())
    }

    // =================================================================
    // The cases a line-local regex cannot get right. These are the whole
    // argument for computing indentation from the parser.
    // =================================================================

    #[test]
    fn a_bracket_inside_a_literal_or_a_comment_opens_nothing() {
        // `$(`, `"("`, `'('` and `// (` are the four the editor's own
        // indentation rules are wrong about. None of them is an `LParen`.
        let out = script(&["(", "$(;", "\")\";", "'(';", "// (", "1;", ")"]);
        assert_eq!(
            out,
            file(&[
                "(",
                "    $(;",
                "    \")\";",
                "    '(';",
                "    // (",
                "    1;",
                ")"
            ]),
            "a literal bracket must not open a block: {out:?}"
        );
    }

    #[test]
    fn block_comments_nest() {
        // SuperCollider's block comments nest, which no line-local regex can
        // track. The lexer already does.
        let out = script(&["(", "/* /* */ */", "1;", ")"]);
        assert_eq!(
            out,
            file(&["(", "    /* /* */ */", "    1;", ")"]),
            "{out:?}"
        );
    }

    #[test]
    fn a_pipe_is_not_a_bracket() {
        // The VS Code grammar lists `|` as a bracket pair for the sake of
        // `|a, b|`, which makes it wrong about `a | b`. Neither indents here.
        let out = script(&["(", "f = { |a, b|", "x;", "};", "a | b;", ")"]);
        assert_eq!(
            out,
            file(&[
                "(",
                "    f = { |a, b|",
                "        x;",
                "    };",
                "    a | b;",
                ")"
            ]),
            "{out:?}"
        );
    }

    #[test]
    fn a_closed_function_opens_one_level() {
        // `#{` is a single token, so counting braces in the text sees nothing.
        let out = script(&["(", "f = #{", "1;", "};", ")"]);
        assert_eq!(
            out,
            file(&["(", "    f = #{", "        1;", "    };", ")"]),
            "{out:?}"
        );
    }

    // =================================================================
    // Structure
    // =================================================================

    #[test]
    fn a_region_body_is_indented() {
        let out = script(&["(", "var x = 1;", "x.postln;", ")"]);
        assert_eq!(
            out,
            file(&["(", "    var x = 1;", "    x.postln;", ")"]),
            "{out:?}"
        );
    }

    #[test]
    fn a_class_nests_its_methods_and_their_bodies() {
        let out = class_file(&["Foo : Bar {", "var x;", "baz {", "^1", "}", "}"]);
        assert_eq!(
            out,
            file(&[
                "Foo : Bar {",
                "    var x;",
                "    baz {",
                "        ^1",
                "    }",
                "}"
            ]),
            "{out:?}"
        );
    }

    #[test]
    fn a_comment_on_its_own_line_moves_with_its_block() {
        let out = script(&["(", "// a note", "1;", ")"]);
        assert_eq!(out, file(&["(", "    // a note", "    1;", ")"]), "{out:?}");
    }

    #[test]
    fn statements_in_a_sequence_are_not_continuations_of_the_first() {
        // `ExprSeq` holds a list of statements rather than being one, so `b`
        // starts a statement rather than continuing `a`.
        let out = script(&["(", "a;", "b;", "c;", ")"]);
        assert_eq!(
            out,
            file(&["(", "    a;", "    b;", "    c;", ")"]),
            "{out:?}"
        );
    }

    // =================================================================
    // What the author aligned by hand
    // =================================================================

    #[test]
    fn alignment_inside_a_line_survives() {
        // The reason this is an indenter and not a formatter. Only leading
        // whitespace is touched, so the value column stays put.
        let out = script(&[
            "(",
            "Pbind(",
            "\\degree, Pseq([0, 2, 4], inf),",
            "\\dur,    0.25",
            ").play;",
            ")",
        ]);
        assert_eq!(
            out,
            file(&[
                "(",
                "    Pbind(",
                "        \\degree, Pseq([0, 2, 4], inf),",
                "        \\dur,    0.25",
                "    ).play;",
                ")",
            ]),
            "{out:?}"
        );
    }

    #[test]
    fn a_continuation_moves_with_its_block_and_keeps_its_offset() {
        let flush = script(&["(", "SinOsc.ar(440)", "+ Saw.ar(220);", ")"]);
        assert_eq!(
            flush,
            file(&["(", "    SinOsc.ar(440)", "    + Saw.ar(220);", ")"]),
            "no offset to keep: {flush:?}"
        );

        let offset = script(&["(", "SinOsc.ar(440)", "    + Saw.ar(220);", ")"]);
        assert_eq!(
            offset,
            file(&["(", "    SinOsc.ar(440)", "        + Saw.ar(220);", ")"]),
            "four columns of offset, kept: {offset:?}"
        );
    }

    #[test]
    fn a_call_split_inside_a_continuation_indents_from_where_it_sits() {
        // The case that makes every answer relative rather than an absolute
        // depth: the argument belongs under `.bar(`, not under the depth its
        // nesting would otherwise give it.
        let out = script(&["(", "foo", "    .bar(", "1", ");", ")"]);
        assert_eq!(
            out,
            file(&[
                "(",
                "    foo",
                "        .bar(",
                "            1",
                "        );",
                ")"
            ]),
            "{out:?}"
        );
    }

    // =================================================================
    // What must not be touched
    // =================================================================

    #[test]
    fn the_inside_of_a_multi_line_string_is_left_alone() {
        // The whole literal is one token, so no token *starts* on its later
        // lines and there is nothing to reindent. Changing them would change
        // the string.
        let out = script(&["(", "x = \"line one", "line two\";", ")"]);
        assert_eq!(
            out,
            file(&["(", "    x = \"line one", "line two\";", ")"]),
            "{out:?}"
        );
    }

    #[test]
    fn blank_lines_stay_blank() {
        let out = script(&["(", "a;", "", "b;", ")"]);
        assert_eq!(out, file(&["(", "    a;", "", "    b;", ")"]), "{out:?}");
    }

    #[test]
    fn crlf_endings_survive() {
        let source = "(\r\nvar x = 1;\r\n)\r\n";
        let out = reindent(source, &parse_script(source), IndentStyle::default());
        assert_eq!(out, "(\r\n    var x = 1;\r\n)\r\n", "{out:?}");
    }

    #[test]
    fn a_file_already_indented_is_unchanged() {
        let source = file(&["(", "    var x = 1;", "    x.postln;", ")"]);
        let out = reindent(&source, &parse_script(&source), IndentStyle::default());
        assert_eq!(out, source, "{out:?}");
    }

    // =================================================================
    // Style and recovery
    // =================================================================

    #[test]
    fn tabs_are_used_when_the_editor_asks_for_them() {
        let source = file(&["(", "var x = 1;", ")"]);
        let style = IndentStyle {
            tab_size: 4,
            insert_spaces: false,
        };
        let out = reindent(&source, &parse_script(&source), style);
        assert_eq!(out, "(\n\tvar x = 1;\n)\n", "{out:?}");
    }

    #[test]
    fn an_unclosed_construct_still_indents_what_it_can() {
        // The usual state of a buffer being typed into. The server declines to
        // format a broken file, but this must stay total regardless.
        let source = file(&["(", "var x = 1;"]);
        let out = reindent(&source, &parse_script(&source), IndentStyle::default());
        assert_eq!(out, file(&["(", "    var x = 1;"]), "{out:?}");
    }

    #[test]
    fn there_is_one_answer_per_line() {
        let source = file(&["(", "a;", "", "b;", ")"]);
        let indents = line_indents(&source, &parse_script(&source));
        // `file` appends a terminator, so the last line is the empty one after
        // it — the same count `LineIndex` reports.
        assert_eq!(indents.len(), 6, "{indents:?}");
    }
}
