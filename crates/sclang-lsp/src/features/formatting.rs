//! Indentation, as text edits.
//!
//! The computation lives in `sclang_syntax::indent`; this decides what to send
//! an editor. Two things shape it.
//!
//! An edit covers a line's leading whitespace and nothing else, so a client
//! applying it moves no line break and disturbs no cursor sitting in the
//! code. A line whose indent is already right produces no edit at all, which
//! is what keeps format-on-save from touching a file's modification state for
//! nothing.
//!
//! And a file that does not parse is left completely alone. The alternative —
//! guessing at the indentation of code whose structure we could not read — is
//! how a formatter eats someone's work, and diagnostics already say the file
//! is broken. Nothing is reported from here, because format-on-save runs on
//! every save of a file someone is still typing into, and a notification each
//! time is noise.

use crate::analysis::visit_tokens;
use crate::documents::Document;
use crate::line_index::PositionEncoding;

use lsp_types::{FormattingOptions, Position, Range, TextEdit};
use sclang_syntax::{indent_columns, leading_whitespace, IndentStyle, SyntaxKind};

/// Re-indent a whole document.
pub fn formatting(
    doc: &Document,
    options: &FormattingOptions,
    enc: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    edits(doc, options, enc, None)
}

/// Re-indent the lines a range touches — the "re-indent this region" the
/// SuperCollider IDE binds to a key.
///
/// The lines outside the range are still *computed*, because what a line
/// should do depends on the ones above it; they are simply not sent.
pub fn range_formatting(
    doc: &Document,
    range: Range,
    options: &FormattingOptions,
    enc: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    // A selection dragged to the start of a line does not include that line.
    let last = if range.end.character == 0 && range.end.line > range.start.line {
        range.end.line - 1
    } else {
        range.end.line
    };
    edits(doc, options, enc, Some((range.start.line, last)))
}

fn edits(
    doc: &Document,
    options: &FormattingOptions,
    enc: PositionEncoding,
    lines: Option<(u32, u32)>,
) -> Option<Vec<TextEdit>> {
    if !readable(doc) {
        return None;
    }

    let style = IndentStyle {
        tab_size: options.tab_size,
        insert_spaces: options.insert_spaces,
    };
    let targets = indent_columns(&doc.text, doc.parse(), style);

    let mut out = Vec::new();
    for (line, target) in targets.iter().enumerate() {
        let line = line as u32;
        if let Some((first, last)) = lines {
            if line < first || line > last {
                continue;
            }
        }
        let Some(columns) = *target else { continue };

        let start = doc
            .line_index
            .offset(&doc.text, Position::new(line, 0), enc);
        let text = line_text(&doc.text, start);
        let (bytes, _) = leading_whitespace(text, style.tab_size);
        // Nothing but whitespace on the line: there is nothing to indent, and
        // trailing whitespace is the editor's business rather than ours.
        if bytes as usize == text.len() {
            continue;
        }

        let new_text = style.render(columns);
        if new_text == text[..bytes as usize] {
            continue;
        }
        out.push(TextEdit {
            range: doc.line_index.range(&doc.text, start..start + bytes, enc),
            new_text,
        });
    }
    Some(out)
}

/// Whether the tree can be trusted to say where things belong.
fn readable(doc: &Document) -> bool {
    let parse = doc.parse();
    if !parse.is_ok() {
        return false;
    }
    // An unterminated string or block comment is one `Error` token running to
    // the end of the file, and the parser does not always have anything to
    // report about it. It is also the case most likely to make an indenter
    // rewrite something that is really a literal.
    let mut damaged = false;
    visit_tokens(&parse.root, &mut |token| {
        damaged |= token.kind == SyntaxKind::Error;
    });
    !damaged
}

/// One line of `text`, starting at `start`, without its terminator.
fn line_text(text: &str, start: u32) -> &str {
    let rest = &text[start as usize..];
    let end = rest.find('\n').unwrap_or(rest.len());
    // A `\r` would otherwise read as part of the leading whitespace of a line
    // holding nothing else, and be replaced along with it.
    rest[..end].strip_suffix('\r').unwrap_or(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sclang_syntax::Mode;

    fn options() -> FormattingOptions {
        FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..Default::default()
        }
    }

    /// The document with its formatting edits applied, so a test reads as the
    /// code it is about. Edits are disjoint and in order, so applying them
    /// back to front needs no offset arithmetic.
    fn formatted(source: &str) -> String {
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        let mut out = source.to_string();
        let edits = formatting(&doc, &options(), PositionEncoding::Utf16).unwrap_or_default();
        for edit in edits.iter().rev() {
            let start = doc
                .line_index
                .offset(&doc.text, edit.range.start, PositionEncoding::Utf16)
                as usize;
            let end = doc
                .line_index
                .offset(&doc.text, edit.range.end, PositionEncoding::Utf16)
                as usize;
            out.replace_range(start..end, &edit.new_text);
        }
        out
    }

    #[test]
    fn a_region_body_is_indented() {
        assert_eq!(
            formatted("(\nvar x = 1;\n)\n"),
            "(\n    var x = 1;\n)\n",
            "the edits should indent the body"
        );
    }

    #[test]
    fn a_file_already_indented_produces_no_edits() {
        let source = "(\n    var x = 1;\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        let edits = formatting(&doc, &options(), PositionEncoding::Utf16).unwrap();
        assert!(edits.is_empty(), "{edits:?}");
    }

    #[test]
    fn only_the_lines_that_change_get_an_edit() {
        let source = "(\nvar x = 1;\n    var y = 2;\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        let edits = formatting(&doc, &options(), PositionEncoding::Utf16).unwrap();
        assert_eq!(edits.len(), 1, "only line 1 is wrong: {edits:?}");
        assert_eq!(edits[0].range.start.line, 1, "{edits:?}");
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        // Unbalanced braces: the tree cannot say where anything belongs.
        let doc = Document::with_mode("(\nvar x = {{{ ;\n)\n".to_string(), 1, Mode::Script);
        assert!(
            formatting(&doc, &options(), PositionEncoding::Utf16).is_none(),
            "a file with syntax errors must produce no edits at all"
        );
    }

    #[test]
    fn an_unterminated_string_is_left_alone() {
        // The lexer makes this one `Error` token running to the end of the
        // file, which the parser has nothing to say about on its own.
        let doc = Document::with_mode("(\nx = \"unterminated;\n)\n".to_string(), 1, Mode::Script);
        assert!(
            formatting(&doc, &options(), PositionEncoding::Utf16).is_none(),
            "an unterminated literal must produce no edits at all"
        );
    }

    #[test]
    fn range_formatting_sends_only_the_lines_asked_for() {
        let source = "(\nvar x = 1;\nvar y = 2;\nvar z = 3;\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        let range = Range::new(Position::new(2, 0), Position::new(2, 10));
        let edits = range_formatting(&doc, range, &options(), PositionEncoding::Utf16).unwrap();
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0].range.start.line, 2, "{edits:?}");
    }

    #[test]
    fn a_selection_stopping_at_a_line_start_excludes_that_line() {
        let source = "(\nvar x = 1;\nvar y = 2;\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        // What an editor sends for "lines 1 to 2 selected".
        let range = Range::new(Position::new(1, 0), Position::new(2, 0));
        let edits = range_formatting(&doc, range, &options(), PositionEncoding::Utf16).unwrap();
        assert_eq!(edits.len(), 1, "line 2 was not selected: {edits:?}");
        assert_eq!(edits[0].range.start.line, 1, "{edits:?}");
    }

    #[test]
    fn tabs_are_used_when_the_editor_asks_for_them() {
        let source = "(\nvar x = 1;\n)\n";
        let doc = Document::with_mode(source.to_string(), 1, Mode::Script);
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: false,
            ..Default::default()
        };
        let edits = formatting(&doc, &options, PositionEncoding::Utf16).unwrap();
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0].new_text, "\t", "{edits:?}");
    }
}
