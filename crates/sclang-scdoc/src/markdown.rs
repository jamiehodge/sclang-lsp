//! Help text as Markdown, for the places an editor shows a few paragraphs:
//! hover, the documentation beside a completion, signature help.
//!
//! That shapes what is kept. Prose and its structure are — lists, tables,
//! notes, inline code. Code blocks are not: in a help file they are examples,
//! often dozens of lines, and a popup is the wrong place for them. Footnotes,
//! images, anchors and keywords are dropped as well, since none of them reads
//! as anything in a popup.
//!
//! Links render as the text SCDoc's own HTML renderer gives them
//! (`htmlForLink`), so `link::Classes/SinOsc::` reads `SinOsc`. Only links
//! outside the help system become Markdown links; an internal one would need
//! a rendered page to point at, and a quark's pages are rendered nowhere.

use crate::{Id, Node};

/// Render a sequence of body nodes — the children of a `DESCRIPTION`, a
/// `METHODBODY`, an `ARGUMENT` — as Markdown, paragraphs separated by blank
/// lines. Empty when nothing in them renders.
pub fn render(nodes: &[Node]) -> String {
    let mut blocks = Vec::new();
    for node in nodes {
        block(node, &mut blocks);
    }
    blocks.join("\n\n")
}

/// The prose of `nodes` as one line of Markdown, for a place that has room for
/// no more: an argument's description, say. Block structure is flattened.
pub fn render_inline(nodes: &[Node]) -> String {
    render(nodes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn block(node: &Node, out: &mut Vec<String>) {
    let rendered = match node.id {
        Id::Prose => inline(&node.children),
        Id::List | Id::Tree => list(node, |_| "- ".to_string()),
        Id::NumberedList => list(node, |i| format!("{}. ", i + 1)),
        Id::DefinitionList => definitions(node),
        Id::Table => table(node),
        Id::Note => callout("Note", &node.children),
        Id::Warning => callout("Warning", &node.children),
        Id::TeletypeBlock | Id::MathBlock => fence(node.text.as_deref().unwrap_or("")),
        Id::Section | Id::Subsection | Id::Subsubsection => {
            if let Some(title) = &node.text {
                out.push(format!("**{}**", escape(title)));
            }
            for child in &node.children {
                block(child, out);
            }
            return;
        }
        // Examples, and what does not read as anything in a popup. Methods
        // documented inside a description belong to their own hover.
        _ => return,
    };
    let rendered = rendered.trim().to_string();
    if !rendered.is_empty() {
        out.push(rendered);
    }
}

fn inline(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        let text = node.text.as_deref().unwrap_or("");
        match node.id {
            Id::Text | Id::Soft => push_words(&mut out, &escape(text)),
            Id::Nl => push_words(&mut out, " "),
            Id::Code | Id::Teletype | Id::Math => out.push_str(&code_span(text)),
            Id::Strong => out.push_str(&format!("**{}**", escape(text))),
            Id::Emphasis => out.push_str(&format!("*{}*", escape(text))),
            Id::Link => out.push_str(&link(text)),
            // Anchors are invisible; footnotes have nowhere to go.
            _ => {}
        }
    }
    out
}

/// Append prose, reading a run of blanks as one as HTML does. Code spans are
/// appended as they are, since their spacing is theirs.
fn push_words(out: &mut String, text: &str) {
    for c in text.chars() {
        let blank = c.is_whitespace();
        if blank && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(if blank { ' ' } else { c });
    }
}

/// `htmlForLink`'s text, as a Markdown link when it leaves the help system.
fn link(target: &str) -> String {
    let mut parts = target.splitn(3, '#');
    let base = parts.next().unwrap_or("");
    let anchor = parts.next().unwrap_or("");
    let label = parts.next().unwrap_or("");

    let external = base.split_once("://").is_some_and(|(scheme, rest)| {
        !scheme.is_empty() && scheme.bytes().all(|b| b.is_ascii_alphabetic()) && !rest.is_empty()
    });
    if external {
        let text = if label.is_empty() { target } else { label };
        let url = if anchor.is_empty() {
            base.to_string()
        } else {
            format!("{base}#{anchor}")
        };
        return format!("[{}](<{}>)", escape(text), url);
    }

    let text = match (label, base, anchor) {
        (label, _, _) if !label.is_empty() => label.to_string(),
        (_, "", "") => return String::new(),
        (_, "", anchor) => anchor.to_string(),
        // The document's title would be better, but it is in a file nobody
        // has asked for; the last path component is what the renderer falls
        // back to, and for a class it is the title anyway.
        (_, base, anchor) => {
            let name = base.rsplit('/').next().unwrap_or(base);
            if anchor.is_empty() {
                name.to_string()
            } else {
                format!("{name}: {anchor}")
            }
        }
    };
    escape(&text)
}

fn list(node: &Node, marker: impl Fn(usize) -> String) -> String {
    node.children
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = marker(i);
            let indent = " ".repeat(marker.len());
            let body = render(&item.children);
            let mut lines = body.lines();
            let mut out = format!("{marker}{}", lines.next().unwrap_or(""));
            for line in lines {
                out.push('\n');
                if !line.is_empty() {
                    out.push_str(&indent);
                    out.push_str(line);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A table as a list of rows. SCDoc has no header row, and a Markdown table
/// needs one; inventing it from the first row is wrong as often as right.
fn table(node: &Node) -> String {
    node.children
        .iter()
        .map(|row| {
            let cells: Vec<String> = row
                .children
                .iter()
                .map(|cell| render_inline(&cell.children))
                .collect();
            format!("- {}", cells.join(" — "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn definitions(node: &Node) -> String {
    node.children
        .iter()
        .map(|item| {
            let terms: Vec<String> = item
                .children
                .iter()
                .filter(|c| c.id == Id::Term)
                .map(|t| format!("**{}**", render_inline(&t.children)))
                .collect();
            let definition = item
                .child(Id::Definition)
                .map(|d| render_inline(&d.children))
                .unwrap_or_default();
            if definition.is_empty() {
                format!("- {}", terms.join(", "))
            } else {
                format!("- {} — {definition}", terms.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn callout(label: &str, children: &[Node]) -> String {
    let body = render(children);
    let mut out = String::new();
    for (i, line) in body.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push('>');
        if i == 0 {
            out.push_str(&format!(" **{label}:**"));
        }
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
    }
    if out.is_empty() {
        out = format!("> **{label}**");
    }
    out
}

/// A fenced block long enough not to be closed by anything inside it.
fn fence(text: &str) -> String {
    let ticks = "`".repeat(longest_run(text, '`').max(2) + 1);
    format!("{ticks}\n{text}\n{ticks}")
}

/// An inline code span, delimited by more backticks than it contains, and
/// padded when it starts or ends with one, as CommonMark requires.
fn code_span(text: &str) -> String {
    let ticks = "`".repeat(longest_run(text, '`') + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

fn longest_run(text: &str, c: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == c {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// Escape what Markdown would otherwise read as markup. Help text is full of
/// `*`, `_` and `<` that mean themselves: `SinOsc.ar * 0.1`, `freq_`,
/// `a < b`. Public for the text that arrives as a plain string, like a
/// summary.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, Mode};

    /// The description of a one-off help file, rendered.
    fn description(body: &str) -> String {
        let src = format!("class:: T\nsummary:: s\ndescription::\n{body}\n");
        let doc = parse(&src, Mode::Full).expect("parses");
        let desc = doc
            .child(Id::Body)
            .and_then(|b| b.child(Id::Description))
            .expect("a description");
        render(&desc.children)
    }

    #[test]
    fn prose_with_inline_markup() {
        assert_eq!(
            description("Use code::SinOsc.ar:: with strong::care:: and emphasis::taste::."),
            "Use `SinOsc.ar` with **care** and *taste*."
        );
    }

    #[test]
    fn line_breaks_inside_a_paragraph_are_spaces() {
        assert_eq!(
            description("A link::Classes/Osc::\nthat wraps."),
            "A Osc that wraps."
        );
    }

    #[test]
    fn paragraphs_stay_apart() {
        assert_eq!(description("One.\n\nTwo."), "One.\n\nTwo.");
    }

    #[test]
    fn links_read_as_sclang_renders_them() {
        assert_eq!(description("link::Classes/SinOsc::"), "SinOsc");
        assert_eq!(description("link::Classes/SinOsc#*ar::"), "SinOsc: \\*ar");
        assert_eq!(description("link::Guides/X#a#the guide::"), "the guide");
        assert_eq!(
            description("link::https://supercollider.github.io::"),
            "[https://supercollider.github.io](<https://supercollider.github.io>)"
        );
        assert_eq!(
            description("See https://example.com/a now."),
            "See [https://example.com/a](<https://example.com/a>) now."
        );
    }

    #[test]
    fn examples_are_left_out() {
        assert_eq!(
            description("Before.\ncode::\n{ SinOsc.ar }.play;\n::\nAfter."),
            "Before.\n\nAfter."
        );
    }

    #[test]
    fn markdown_characters_mean_themselves() {
        assert_eq!(description("a * b_c <d>"), "a \\* b\\_c \\<d\\>");
        assert_eq!(description("code::a `b`::"), "`` a `b` ``");
    }

    #[test]
    fn lists_tables_and_definitions() {
        assert_eq!(description("list::\n## one\n## two\n::"), "- one\n- two");
        assert_eq!(
            description("numberedlist::\n## one\n## two\n::"),
            "1. one\n2. two"
        );
        assert_eq!(
            description("table::\n## a || b\n## c || d\n::"),
            "- a — b\n- c — d"
        );
        assert_eq!(
            description("definitionlist::\n## freq || In Hertz.\n::"),
            "- **freq** — In Hertz."
        );
    }

    #[test]
    fn notes_are_quoted() {
        assert_eq!(
            description("note:: Mind the gap. ::"),
            "> **Note:** Mind the gap."
        );
    }

    #[test]
    fn subsections_become_bold_headings() {
        assert_eq!(
            description("Intro.\nsubsection:: More\nDetail."),
            "Intro.\n\n**More**\n\nDetail."
        );
    }
}
