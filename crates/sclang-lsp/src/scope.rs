//! Names visible at a point that the symbol index knows nothing about.
//!
//! The index covers classes and methods, which is the whole workspace. It has
//! no notion of a function body, so the arguments and variables a user has
//! just written were invisible to completion — and worse than invisible, since
//! the list filled with globals that merely shared a prefix.
//!
//! Everything here comes from the syntax tree of the open buffer alone. There
//! is nothing to index and nothing to invalidate.

use crate::analysis::ancestors_at;
use sclang_syntax::{SyntaxKind, SyntaxNode};

/// Where a visible name came from. Shapes how it is offered and ranked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// A function or method parameter.
    Argument,
    /// A `var` inside a function body.
    Variable,
    /// A `var` on a class: an instance variable.
    InstanceVar,
    /// A `classvar`.
    ClassVar,
    /// A `const`.
    Constant,
}

impl LocalKind {
    pub fn describe(self) -> &'static str {
        match self {
            LocalKind::Argument => "argument",
            LocalKind::Variable => "variable",
            LocalKind::InstanceVar => "instance variable",
            LocalKind::ClassVar => "classvar",
            LocalKind::Constant => "const",
        }
    }
}

/// A name in scope, with where it came from and its default if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub kind: LocalKind,
    pub default: Option<String>,
}

/// The pseudo-variables sclang resolves during compilation.
///
/// `PyrLexer.cpp` returns these as plain identifiers — there is no token kind
/// for them — so nothing else in this crate would ever mention them, and a
/// user typing `thi` should still be offered them.
const PSEUDO_VARIABLES: &[&str] = &[
    "this",
    "super",
    "thisProcess",
    "thisThread",
    "thisMethod",
    "thisFunction",
    "thisFunctionDef",
];

/// Every name visible at `offset`, innermost scope first.
///
/// A name declared in an inner scope shadows the same name outside it, so the
/// first of any duplicate pair is the one that wins — exactly as it would at
/// run time.
pub fn locals_at(root: &SyntaxNode, source: &str, offset: u32) -> Vec<Local> {
    let mut out: Vec<Local> = Vec::new();

    // Outermost first from the walk, so reverse to put the innermost scope —
    // the one whose declarations shadow — at the front.
    for node in ancestors_at(root, offset).into_iter().rev() {
        match node.kind {
            SyntaxKind::MethodDef | SyntaxKind::FunctionBlock => {
                collect_declarations(node, source, &mut out);
            }
            SyntaxKind::ClassDef | SyntaxKind::ClassExtension => {
                collect_class_slots(node, source, &mut out);
            }
            _ => {}
        }
    }

    for name in PSEUDO_VARIABLES {
        push_unique(
            &mut out,
            Local {
                name: (*name).to_string(),
                kind: LocalKind::Variable,
                default: None,
            },
        );
    }

    out
}

/// `arg`/`|...|` parameters and `var` declarations of one body.
fn collect_declarations(node: &SyntaxNode, source: &str, out: &mut Vec<Local>) {
    for child in node.child_nodes() {
        let kind = match child.kind {
            SyntaxKind::ArgDecls => LocalKind::Argument,
            SyntaxKind::VarDecls => LocalKind::Variable,
            _ => continue,
        };
        for def in child.child_nodes() {
            // `...rest` is a parameter too, and carries no default.
            if !matches!(def.kind, SyntaxKind::VarDef | SyntaxKind::RestArg) {
                continue;
            }
            if let Some(name) = def.token_of(SyntaxKind::Ident) {
                push_unique(
                    out,
                    Local {
                        name: name.text(source).to_string(),
                        kind,
                        default: default_of(def, source),
                    },
                );
            }
        }
    }
}

/// `var`, `classvar` and `const` declared on a class.
fn collect_class_slots(node: &SyntaxNode, source: &str, out: &mut Vec<Local>) {
    for decl in node.child_nodes() {
        if decl.kind != SyntaxKind::ClassVarDecl {
            continue;
        }
        let kind = if decl.token_of(SyntaxKind::ClassvarKw).is_some() {
            LocalKind::ClassVar
        } else if decl.token_of(SyntaxKind::ConstKw).is_some() {
            LocalKind::Constant
        } else {
            LocalKind::InstanceVar
        };

        for slot in decl.child_nodes() {
            if slot.kind != SyntaxKind::SlotDef {
                continue;
            }
            if let Some(name) = slot.token_of(SyntaxKind::Ident) {
                push_unique(
                    out,
                    Local {
                        name: name.text(source).to_string(),
                        kind,
                        default: default_of(slot, source),
                    },
                );
            }
        }
    }
}

/// The source text of a declaration's default value, if it has one.
fn default_of(def: &SyntaxNode, source: &str) -> Option<String> {
    let eq = def.token_of(SyntaxKind::Eq)?;
    let value = def
        .children
        .iter()
        .find(|c| c.range().0 >= eq.end && !c.kind().is_trivia())?;
    let (start, end) = value.range();
    Some(source[start as usize..end as usize].trim().to_string())
}

/// Keep the first sighting of a name: the innermost declaration shadows.
fn push_unique(out: &mut Vec<Local>, local: Local) {
    if !out.iter().any(|l| l.name == local.name) {
        out.push(local);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sclang_syntax::parse;

    fn locals(source: &str, offset: u32) -> Vec<Local> {
        let parse = parse(source);
        locals_at(&parse.root, source, offset)
    }

    fn names(source: &str, offset: u32) -> Vec<String> {
        locals(source, offset)
            .into_iter()
            .filter(|l| !PSEUDO_VARIABLES.contains(&l.name.as_str()))
            .map(|l| l.name)
            .collect()
    }

    #[test]
    fn method_arguments_and_variables() {
        // Cursor inside the body, after `^`.
        let source = "Thing { make { |freq = 440, phase| var scaled = 1, other; ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        assert_eq!(
            names(source, offset),
            vec!["freq", "phase", "scaled", "other"]
        );
    }

    #[test]
    fn defaults_are_captured() {
        let source = "T { m { |freq = 440| ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        let freq = locals(source, offset)
            .into_iter()
            .find(|l| l.name == "freq")
            .unwrap();
        assert_eq!(freq.default.as_deref(), Some("440"));
        assert_eq!(freq.kind, LocalKind::Argument);
    }

    #[test]
    fn class_slots_are_visible_in_methods() {
        let source = "T { var <count; classvar all; const max = 9; m { ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        let found = locals(source, offset);
        let kind_of = |n: &str| found.iter().find(|l| l.name == n).map(|l| l.kind);
        assert_eq!(kind_of("count"), Some(LocalKind::InstanceVar));
        assert_eq!(kind_of("all"), Some(LocalKind::ClassVar));
        assert_eq!(kind_of("max"), Some(LocalKind::Constant));
    }

    #[test]
    fn nested_blocks_see_both_scopes_innermost_first() {
        let source = "{ |outer| { |inner| var deep; x } }";
        let offset = source.rfind('x').unwrap() as u32;
        // Innermost declarations lead, which is what shadowing requires.
        assert_eq!(names(source, offset), vec!["inner", "deep", "outer"]);
    }

    #[test]
    fn an_inner_name_shadows_an_outer_one() {
        let source = "{ |v| { |v| x } }";
        let offset = source.rfind('x').unwrap() as u32;
        assert_eq!(names(source, offset), vec!["v"]);
    }

    #[test]
    fn nothing_leaks_from_a_sibling_method() {
        let source = "T { a { |mine| ^1 } b { ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        assert!(!names(source, offset).contains(&"mine".to_string()));
    }

    #[test]
    fn works_in_an_unclosed_block() {
        // The state a buffer is in while being typed: no closing brace yet.
        let source = "T { m { |freq| ^fr";
        let offset = source.len() as u32;
        assert_eq!(names(source, offset), vec!["freq"]);
    }

    #[test]
    fn pseudo_variables_are_offered() {
        let source = "T { m { ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        let all: Vec<String> = locals(source, offset).into_iter().map(|l| l.name).collect();
        assert!(all.contains(&"this".to_string()));
        assert!(all.contains(&"thisProcess".to_string()));
    }

    #[test]
    fn rest_argument_is_a_parameter() {
        let source = "T { m { |a ...rest| ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        assert_eq!(names(source, offset), vec!["a", "rest"]);
    }
}
