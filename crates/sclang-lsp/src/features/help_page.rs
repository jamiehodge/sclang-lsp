//! A class's whole help page, as a SuperCollider file to read and run.
//!
//! The help browser's job, done in the editor: prose becomes comments, and
//! the examples stay code, so evaluating one is the keystroke it always is.
//! The page is written to a cache directory and opened there; it is rewritten
//! each time it is asked for, so it never goes stale and nothing typed into it
//! is kept.
//!
//! Examples are changed in one way only, and only when they could not
//! otherwise be run as they stand. A help file's `code::` block is shown in a
//! browser, where a reader selects what to run. In an editor the unit is the
//! line, or the parenthesised region around the cursor — so a statement
//! written across several lines, or one that declares `var`s, which sclang
//! scopes to a single evaluation, is wrapped in `( … )`. A block of separate
//! statements is left alone: those are meant to be stepped through one line
//! at a time. Code that does not parse is not code anyone can run, and is
//! kept as a comment — judged a paragraph at a time when the block as a whole
//! fails, since a block of pseudo-code often has a runnable line or two after
//! it. Only a paragraph that starts at the margin is judged alone: one that
//! starts indented is the inside of something, a class body as often as not,
//! and means nothing out of it.
//!
//! One more change is about the page rather than the example. Read as one
//! file, an example ending `k[4].value` runs into a following one opening
//! with `(`, which parses as a call and lights the page with an error neither
//! example has. A last statement with no `;` gets one, which changes nothing
//! about evaluating it.

use crate::analysis::{point_at, resolve_selector, Bias, Point};
use crate::documents::Document;
use sclang_index::{copy_target, MethodKind, SymbolIndex};
use sclang_scdoc::{Id, Mode, Node};
use sclang_syntax::SyntaxKind;
use std::path::{Path, PathBuf};

/// The width prose is wrapped to, comment markers included.
const WIDTH: usize = 80;

/// What to open: a class's page, and optionally one entry on it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default)]
    pub class_side: bool,
}

impl Target {
    pub fn describe(&self) -> String {
        match &self.method {
            Some(m) if self.class_side => format!("{}.{m}", self.class),
            Some(m) => format!("{}-{m}", self.class),
            None => self.class.clone(),
        }
    }

    fn kind(&self) -> MethodKind {
        if self.class_side {
            MethodKind::Class
        } else {
            MethodKind::Instance
        }
    }
}

/// The page to open for whatever is under the cursor, if it has one.
///
/// A class name is its own page. A selector is its owner's, when dispatch
/// narrows it to one method: which of several classes' pages was meant is a
/// guess, and opening the wrong one is worse than offering nothing.
pub fn target_at(doc: &Document, index: &SymbolIndex, offset: u32) -> Option<Target> {
    let root = &doc.parse().root;
    let target = match point_at(root, &doc.text, offset, Bias::Inside) {
        Point::ClassName(class) => Target {
            class,
            method: None,
            class_side: false,
        },
        Point::Selector { name, receiver } => {
            let m = resolve_selector(index, &name, &receiver).single()?;
            Target {
                class: m.owner.clone(),
                method: Some(m.name.clone()),
                class_side: m.kind == MethodKind::Class,
            }
        }
        Point::MethodName { name, owner } => {
            let owner = owner?;
            let class_side = index.method(&owner, &name, MethodKind::Instance).is_none()
                && index.method(&owner, &name, MethodKind::Class).is_some();
            Target {
                class: owner,
                method: Some(name),
                class_side,
            }
        }
        Point::Local { .. } | Point::Nothing => return None,
    };
    index
        .help()
        .class(&target.class)
        .is_some()
        .then_some(target)
}

/// A rendered page.
#[derive(Debug)]
pub struct Page {
    pub text: String,
    /// Where the page should open: the line of the asked-for entry.
    pub line: Option<u32>,
    /// Where it should be written, relative to the cache: `Classes/Foo.scd`.
    pub relative: PathBuf,
}

/// Render a class's page, reading its files afresh. `None` if it has none, or
/// none that can be read now.
pub fn page(index: &SymbolIndex, target: &Target) -> Option<Page> {
    let files = index.help().files(&target.class);
    let mut w = Writer {
        index,
        class: &target.class,
        target,
        lines: Vec::new(),
        found: None,
    };

    let mut any = false;
    for (path, addition) in files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mode = if addition { Mode::Partial } else { Mode::Full };
        let Ok(tree) = sclang_scdoc::parse(&bytes, mode) else {
            continue;
        };
        if addition {
            w.blank();
            w.section(&format!("Added by {}", path.display()));
            w.blocks(&tree.children);
        } else {
            w.document(&tree, path);
        }
        any = true;
    }
    if !any {
        return None;
    }

    let mut text = w.lines.join("\n");
    text.push('\n');
    Some(Page {
        text,
        line: w.found,
        relative: Path::new("Classes").join(format!("{}.scd", target.class)),
    })
}

/// An example as it should appear on the page. See the module comment.
pub fn example(code: &str) -> String {
    let code = code.trim_end();
    if sclang_syntax::parse_script(code).is_ok() {
        return runnable(code);
    }
    let paragraphs = paragraphs(code);
    if paragraphs.len() == 1 {
        return commented(code);
    }
    paragraphs
        .into_iter()
        .map(|p| {
            let at_margin = !p.starts_with([' ', '\t']);
            if at_margin && sclang_syntax::parse_script(p).is_ok() {
                runnable(p)
            } else {
                commented(p)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Runs of non-blank lines.
fn paragraphs(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let blank = line.trim().is_empty();
        match (blank, start) {
            (false, None) => start = Some(offset),
            (true, Some(s)) => {
                out.push(code[s..end].trim_end());
                start = None;
            }
            _ => {}
        }
        offset += line.len();
        if !blank {
            end = offset;
        }
    }
    if let Some(s) = start {
        out.push(code[s..end].trim_end());
    }
    out
}

fn commented(code: &str) -> String {
    code.lines()
        .map(|l| {
            if l.trim().is_empty() {
                "//".to_string()
            } else {
                format!("// {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Code that parses, made ready to run where it is.
fn runnable(code: &str) -> String {
    let parse = sclang_syntax::parse_script(code);
    let top: Vec<_> = parse.root.child_nodes().collect();
    let spans_lines = |n: &&sclang_syntax::SyntaxNode| n.text(code).contains('\n');
    let wrap = match top.as_slice() {
        [] => false,
        [one] => one.kind != SyntaxKind::ParenExpr && spans_lines(one),
        many => many.iter().any(|n| n.kind == SyntaxKind::VarDecls),
    };
    if wrap {
        return format!("(\n{code}\n)");
    }

    // Terminate the last statement, unless it is a region, which ends itself.
    let Some(last) = top.last().filter(|n| n.kind != SyntaxKind::ParenExpr) else {
        return code.to_string();
    };
    // The `;` can be inside the node, as a `var` declaration's is.
    let terminated = last.text(code).trim_end().ends_with(';')
        || parse
            .root
            .child_tokens()
            .any(|t| t.kind == SyntaxKind::Semicolon && t.start >= last.end);
    if terminated {
        code.to_string()
    } else {
        let at = last.end as usize;
        format!("{};{}", &code[..at], &code[at..])
    }
}

struct Writer<'a> {
    index: &'a SymbolIndex,
    class: &'a str,
    target: &'a Target,
    lines: Vec<String>,
    /// The line the target's entry starts on, once written.
    found: Option<u32>,
}

impl Writer<'_> {
    fn document(&mut self, tree: &Node, path: &Path) {
        let header = tree.child(Id::Header);
        let field = |id| {
            header
                .and_then(|h| h.child(id))
                .and_then(|n| n.text.clone())
        };
        let list = |id| -> Vec<String> {
            header
                .and_then(|h| h.child(id))
                .map(|n| n.children.iter().filter_map(|c| c.text.clone()).collect())
                .unwrap_or_default()
        };

        let title = field(Id::Title).unwrap_or_else(|| self.class.to_string());
        match field(Id::Summary) {
            Some(summary) => self.comment(&format!("{title}: {summary}"), "", ""),
            None => self.comment(&title, "", ""),
        }
        let categories = list(Id::Categories);
        if !categories.is_empty() {
            self.comment(&format!("Categories: {}", categories.join(", ")), "", "  ");
        }
        let related: Vec<String> = list(Id::Related).iter().map(|r| link_text(r)).collect();
        if !related.is_empty() {
            self.comment(&format!("See also: {}", related.join(", ")), "", "  ");
        }
        self.line("//".to_string());
        self.comment(
            &format!(
                "The help page from {}, with its examples ready to evaluate. \
                 This file is rewritten each time the page is opened, so \
                 changes made here are not kept.",
                path.display()
            ),
            "",
            "",
        );

        if let Some(body) = tree.child(Id::Body) {
            self.blocks(&body.children);
        }
    }

    fn blocks(&mut self, nodes: &[Node]) {
        for node in nodes {
            self.block(node, "");
        }
    }

    fn block(&mut self, node: &Node, indent: &str) {
        let text = node.text.as_deref().unwrap_or("");
        match node.id {
            Id::Description | Id::ClassMethods | Id::InstanceMethods | Id::Examples => {
                self.blank();
                self.section(match node.id {
                    Id::Description => "Description",
                    Id::ClassMethods => "Class methods",
                    Id::InstanceMethods => "Instance methods",
                    _ => "Examples",
                });
                self.children(node, indent);
            }
            Id::Section => {
                self.blank();
                self.section(text);
                self.children(node, indent);
            }
            Id::Subsection | Id::Subsubsection => {
                self.blank();
                let rule = if node.id == Id::Subsection {
                    "----"
                } else {
                    "~~~~"
                };
                self.line(format!("// {rule} {text} {rule}"));
                self.children(node, indent);
            }
            Id::CMethod | Id::IMethod | Id::Method => self.method(node),
            Id::CCopyMethod | Id::ICopyMethod => self.copied(node),
            Id::Prose => {
                // Upstream makes an empty paragraph of a line break that
                // follows `method::`; it has nothing to say.
                let text = inline(&node.children);
                if !text.is_empty() {
                    self.blank();
                    self.comment(&text, indent, indent);
                }
            }
            Id::CodeBlock => {
                self.blank();
                for line in example(text).lines() {
                    self.line(line.to_string());
                }
                self.blank();
            }
            Id::TeletypeBlock | Id::MathBlock => {
                self.blank();
                for line in text.lines() {
                    self.line(format!("//     {line}").trim_end().to_string());
                }
            }
            Id::List | Id::Tree | Id::NumberedList => {
                self.blank();
                for (i, item) in node.children.iter().enumerate() {
                    let marker = if node.id == Id::NumberedList {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".to_string()
                    };
                    self.item(item, indent, &marker);
                }
            }
            Id::Table => {
                self.blank();
                for row in &node.children {
                    let cells: Vec<String> =
                        row.children.iter().map(|c| prose(&c.children)).collect();
                    self.comment(
                        &cells.join(" | "),
                        &format!("{indent}  "),
                        &format!("{indent}    "),
                    );
                }
            }
            Id::DefinitionList => {
                self.blank();
                for item in &node.children {
                    let terms: Vec<String> = item
                        .children
                        .iter()
                        .filter(|c| c.id == Id::Term)
                        .map(|t| prose(&t.children))
                        .collect();
                    let definition = item
                        .child(Id::Definition)
                        .map(|d| prose(&d.children))
                        .unwrap_or_default();
                    self.comment(
                        &format!("{}: {definition}", terms.join(", ")),
                        &format!("{indent}  "),
                        &format!("{indent}    "),
                    );
                }
            }
            Id::Note | Id::Warning => {
                self.blank();
                let label = if node.id == Id::Note {
                    "Note"
                } else {
                    "Warning"
                };
                let mut first = true;
                for child in &node.children {
                    if child.id == Id::Prose && first {
                        self.comment(
                            &format!("{label}: {}", inline(&child.children)),
                            indent,
                            indent,
                        );
                        first = false;
                    } else {
                        self.block(child, indent);
                    }
                }
                if first {
                    self.comment(label, indent, indent);
                }
            }
            // Images, anchors, keywords, the class tree, private methods and
            // a page-level `copymethod::` have nothing to show in a file.
            _ => {}
        }
    }

    fn children(&mut self, node: &Node, indent: &str) {
        for child in &node.children {
            self.block(child, indent);
        }
    }

    /// A list item: its prose after the marker, anything else beneath.
    fn item(&mut self, item: &Node, indent: &str, marker: &str) {
        let first = format!("{indent}  {marker}");
        let rest = format!("{indent}  {}", " ".repeat(marker.len()));
        let mut prose_done = false;
        for child in &item.children {
            if child.id == Id::Prose && !prose_done {
                self.comment(&inline(&child.children), &first, &rest);
                prose_done = true;
            } else {
                self.block(child, &rest);
            }
        }
        if !prose_done {
            self.comment("", &first, &rest);
        }
    }

    /// A method entry: its signatures, then its text.
    fn method(&mut self, node: &Node) {
        let names: Vec<String> = node
            .child(Id::MethodNames)
            .map(|n| n.children.iter().filter_map(|s| s.text.clone()).collect())
            .unwrap_or_default();
        let kind = match node.id {
            Id::CMethod => Some(MethodKind::Class),
            Id::IMethod => Some(MethodKind::Instance),
            _ => None,
        };
        self.blank();
        self.mark_if_target(&names, kind);
        for name in &names {
            let sig = self.signature(name, kind, node.text.as_deref());
            self.line(format!("// {sig}"));
        }
        if let Some(body) = node.child(Id::MethodBody) {
            self.method_body(&body.children);
        }
    }

    /// A `copymethod::` entry, with the text it copies.
    fn copied(&mut self, node: &Node) {
        let kind = if node.id == Id::CCopyMethod {
            MethodKind::Class
        } else {
            MethodKind::Instance
        };
        let Some((from, name, from_kind)) = copy_target(node.text.as_deref().unwrap_or("")) else {
            return;
        };
        let names = [name.clone()];
        self.blank();
        self.mark_if_target(&names, Some(kind));
        let sig = self.signature(&name, Some(kind), None);
        self.line(format!("// {sig}"));
        let index = self.index;
        match index.help().method(&from, &name, from_kind) {
            Some(help) => self.method_body(help.body),
            None => {
                self.blank();
                self.comment(&format!("As {from}'s, which has no entry for it."), "", "");
            }
        }
    }

    fn method_body(&mut self, body: &[Node]) {
        for child in body {
            match child.id {
                Id::Arguments => {
                    self.blank();
                    self.line("// Arguments:".to_string());
                    for arg in &child.children {
                        let name = arg.text.as_deref().unwrap_or("").trim();
                        let text = prose(&arg.children);
                        let label = if name.is_empty() {
                            text
                        } else {
                            format!("{name}: {text}")
                        };
                        self.comment(&label, "  ", "      ");
                        // Whatever is not prose — a note, a list — goes
                        // beneath it, without the blank lines that would part
                        // it from the next argument. An example keeps them,
                        // since code needs them to read as a region.
                        let from = self.lines.len();
                        let mut has_code = false;
                        for block in arg.children.iter().filter(|b| b.id != Id::Prose) {
                            has_code |= block.id == Id::CodeBlock;
                            self.block(block, "      ");
                        }
                        if !has_code {
                            let tail = self.lines.split_off(from);
                            self.lines
                                .extend(tail.into_iter().filter(|l| !l.is_empty()));
                        }
                    }
                }
                Id::Returns => {
                    self.blank();
                    self.comment(&format!("Returns: {}", prose(&child.children)), "", "  ");
                    for block in child.children.iter().filter(|b| b.id != Id::Prose) {
                        self.block(block, "  ");
                    }
                }
                Id::Discussion => {
                    self.blank();
                    self.line("// Discussion:".to_string());
                    self.children(child, "");
                }
                _ => self.block(child, ""),
            }
        }
    }

    /// `SinOsc.ar(freq = 440, …)` or `.freq` as the help browser writes
    /// them, with the arguments from the class library where it has them.
    fn signature(&self, name: &str, kind: Option<MethodKind>, args: Option<&str>) -> String {
        let prefix = match kind {
            Some(MethodKind::Class) => format!("{}.", self.class),
            Some(MethodKind::Instance) => ".".to_string(),
            None => String::new(),
        };
        let from_index = kind
            .and_then(|k| {
                self.index
                    .methods_visible_on(self.class, k)
                    .into_iter()
                    .find(|m| m.name == name)
            })
            .map(|m| {
                let sig = m.signature();
                sig.trim_start_matches('*')
                    .strip_prefix(name)
                    .unwrap_or("")
                    .to_string()
            });
        let args = from_index
            .filter(|a| a != "()")
            .or_else(|| args.map(str::to_string))
            .unwrap_or_default();
        format!("{prefix}{name}{args}")
    }

    fn mark_if_target(&mut self, names: &[String], kind: Option<MethodKind>) {
        let Some(wanted) = &self.target.method else {
            return;
        };
        let wanted = wanted
            .strip_suffix('_')
            .filter(|g| !g.is_empty())
            .unwrap_or(wanted);
        if self.found.is_none()
            && kind == Some(self.target.kind())
            && names.iter().any(|n| n == wanted)
        {
            self.found = Some(self.lines.len() as u32);
        }
    }

    fn section(&mut self, title: &str) {
        self.line(format!("// ==== {title} ===="));
    }

    /// Wrapped prose as comment lines, the first prefixed with `first` and
    /// the rest with `rest`.
    fn comment(&mut self, text: &str, first: &str, rest: &str) {
        for line in wrap(text, first, rest) {
            self.line(if line.is_empty() {
                "//".to_string()
            } else {
                format!("// {line}")
            });
        }
    }

    fn line(&mut self, line: String) {
        self.lines.push(line);
    }

    /// One blank line, never two, and none at the very top.
    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.is_empty()) {
            self.lines.push(String::new());
        }
    }
}

/// Prose as plain text, inline markup rendered as it reads.
fn inline(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        let text = node.text.as_deref().unwrap_or("");
        match node.id {
            Id::Text | Id::Soft | Id::Strong | Id::Emphasis | Id::Math => out.push_str(text),
            Id::Nl => out.push(' '),
            Id::Code | Id::Teletype => {
                out.push('`');
                out.push_str(text);
                out.push('`');
            }
            Id::Link => out.push_str(&link_text(text)),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every paragraph of a body, run together: for the places that have one
/// line to fill, like an argument.
fn prose(nodes: &[Node]) -> String {
    nodes
        .iter()
        .filter(|n| n.id == Id::Prose)
        .map(|n| inline(&n.children))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A link as SCDoc's HTML renderer labels it. See `markdown::link`.
fn link_text(target: &str) -> String {
    let mut parts = target.splitn(3, '#');
    let base = parts.next().unwrap_or("");
    let anchor = parts.next().unwrap_or("");
    let label = parts.next().unwrap_or("");
    if !label.is_empty() {
        return label.to_string();
    }
    if base.contains("://") {
        return target.to_string();
    }
    let name = base.rsplit('/').next().unwrap_or(base);
    match (name.is_empty(), anchor.is_empty()) {
        (true, _) => anchor.to_string(),
        (false, true) => name.to_string(),
        (false, false) => format!("{name}: {anchor}"),
    }
}

/// Greedy word wrap to [`WIDTH`], counting the `// ` each line gets.
fn wrap(text: &str, first: &str, rest: &str) -> Vec<String> {
    let room = WIDTH - 3;
    let mut lines = Vec::new();
    let mut current = first.to_string();
    let mut empty = true;
    for word in text.split_whitespace() {
        let fits = current.chars().count() + usize::from(!empty) + word.chars().count() <= room;
        if !empty && !fits {
            lines.push(current);
            current = rest.to_string();
            empty = true;
        }
        if !empty {
            current.push(' ');
        }
        current.push_str(word);
        empty = false;
    }
    lines.push(current.trim_end().to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_across_lines_is_wrapped() {
        let code = "Pbind(\n\t\\dur, 0.2\n).play;";
        assert_eq!(example(code), format!("(\n{code}\n)"));
    }

    #[test]
    fn statements_to_step_through_are_left_alone() {
        let code = "s.options.maxLogins = 8;\ns.reboot;";
        assert_eq!(example(code), code);
    }

    #[test]
    fn a_region_already_in_parentheses_is_left_alone() {
        let code = "(\nx = 1;\ny = 2;\n)";
        assert_eq!(example(code), code);
        let two = "(\nx = 1;\n)\n(\ny = 2;\n)";
        assert_eq!(example(two), two);
    }

    #[test]
    fn variables_need_one_evaluation() {
        let code = "var a = 1;\na.postln;";
        assert_eq!(example(code), format!("(\n{code}\n)"));
    }

    #[test]
    fn one_line_is_already_runnable() {
        assert_eq!(example("{ SinOsc.ar }.play;"), "{ SinOsc.ar }.play;");
    }

    #[test]
    fn a_last_statement_is_terminated_so_the_next_example_stays_apart() {
        assert_eq!(
            example("k = 1;\nk.value // the result"),
            "k = 1;\nk.value; // the result"
        );
        // Which is what lets the page parse as a whole.
        let page = format!("{}\n\n{}", example("k.value"), example("(\nvar w;\n)"));
        assert!(sclang_syntax::parse_script(&page).is_ok(), "{page}");
    }

    #[test]
    fn what_does_not_parse_is_commented_out() {
        assert_eq!(example("x = Score([ ... ]);"), "// x = Score([ ... ]);");
    }

    #[test]
    fn the_inside_of_a_class_stays_a_comment() {
        let code = "Foo : ObjectGui {\n\n    var numberEditor;\n\n    guiBody { |layout| ... }\n}";
        assert!(
            example(code)
                .lines()
                .all(|l| l.is_empty() || l.starts_with("//")),
            "{}",
            example(code)
        );
    }

    #[test]
    fn a_declaration_is_not_terminated_twice() {
        assert_eq!(example("var a;"), "var a;");
    }

    #[test]
    fn a_runnable_paragraph_survives_pseudo_code_beside_it() {
        assert_eq!(
            example("~a = Pbind(\\freq, ...);\n\n~a.refresh; ~a.wakeUp;"),
            "// ~a = Pbind(\\freq, ...);\n\n~a.refresh; ~a.wakeUp;"
        );
    }

    #[test]
    fn prose_wraps_inside_the_width() {
        let text = "word ".repeat(40);
        for line in wrap(&text, "", "  ") {
            assert!(line.chars().count() + 3 <= WIDTH, "{line:?}");
        }
    }
}
