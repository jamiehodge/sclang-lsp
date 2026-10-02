//! What to say about a class or a method, as Markdown.
//!
//! The help file when there is one, and the comment above the definition when
//! there is not. A help page is what the class's authors wrote for its users;
//! a comment in the class library is usually a note to whoever maintains it,
//! and the stock library has very few of either kind. Help wins.
//!
//! Hover, completion and signature help all ask, which is why the answers are
//! assembled here once rather than in each.

use sclang_index::{Class, Method, MethodHelp, SymbolIndex};
use sclang_scdoc::markdown::{escape, render};

/// A class's summary and description.
pub fn class(index: &SymbolIndex, class: &Class) -> Option<String> {
    class_by_name(index, &class.name).or_else(|| class.doc.clone())
}

/// A class's summary and description, for a class known only by name.
pub fn class_by_name(index: &SymbolIndex, name: &str) -> Option<String> {
    let help = index.help().class(name)?;
    let parts: Vec<String> = [help.summary.map(escape), Some(render(help.description))]
        .into_iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Everything a method's entry says: what it does, its arguments, what it
/// returns, and the discussion. Code blocks are left out, as everywhere
/// outside the help browser.
pub fn method(index: &SymbolIndex, m: &Method) -> Option<String> {
    let Some(help) = method_help(index, m) else {
        return m.doc.clone();
    };
    let mut parts = Vec::new();
    push(&mut parts, render(&help.prose()));

    let args: Vec<String> = help
        .arguments()
        .map(|(name, body)| (name, render(body)))
        .filter(|(_, text)| !text.is_empty())
        .map(|(name, text)| list_item(&format!("`{}` — {text}", name.trim())))
        .collect();
    if !args.is_empty() {
        parts.push(format!("**Arguments**\n\n{}", args.join("\n")));
    }
    if let Some(returns) = help.returns().map(render).filter(|r| !r.is_empty()) {
        parts.push(format!("**Returns** {returns}"));
    }
    if let Some(discussion) = help.discussion() {
        push(&mut parts, render(discussion));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// What a method does, without its arguments: for signature help, which
/// describes each argument separately.
pub fn method_summary(index: &SymbolIndex, m: &Method) -> Option<String> {
    match method_help(index, m) {
        Some(help) => Some(render(&help.prose())).filter(|s| !s.is_empty()),
        None => m.doc.clone(),
    }
}

/// One argument's description. Block structure is kept: an argument's text
/// is often a sentence and then a `note::`, and both protocols that show it
/// render Markdown.
pub fn argument(index: &SymbolIndex, m: &Method, name: &str) -> Option<String> {
    let help = method_help(index, m)?;
    Some(render(help.argument(name)?)).filter(|s| !s.is_empty())
}

/// A bullet whose later paragraphs stay inside it.
fn list_item(text: &str) -> String {
    let mut out = String::from("- ");
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str("  ");
            }
        }
        out.push_str(line);
    }
    out
}

fn method_help<'a>(index: &'a SymbolIndex, m: &Method) -> Option<MethodHelp<'a>> {
    index.help().method(&m.owner, &m.name, m.kind)
}

fn push(parts: &mut Vec<String>, part: String) {
    if !part.is_empty() {
        parts.push(part);
    }
}
