//! Names visible at a point that the symbol index knows nothing about.
//!
//! The index covers classes and methods, which is the whole workspace. It has
//! no notion of a function body, so the arguments and variables a user has
//! just written were invisible to completion — and worse than invisible, since
//! the list filled with globals that merely shared a prefix.
//!
//! Lexical scope comes from the syntax tree of the open buffer alone: there is
//! nothing to index and nothing to invalidate. One kind of name escapes that,
//! and it is the last part of this module — a class's slots are visible to
//! every subclass, which is inheritance rather than nesting, so finding them
//! means asking the index.

use crate::analysis::ancestors_at;
use sclang_index::{SymbolIndex, Var, VarKind};
use sclang_syntax::{SyntaxKind, SyntaxNode};
use std::ops::Range;

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
    /// The same three-way split the index records on a class slot.
    pub fn of_slot(kind: VarKind) -> Self {
        match kind {
            VarKind::Instance => LocalKind::InstanceVar,
            VarKind::Class => LocalKind::ClassVar,
            VarKind::Const => LocalKind::Constant,
        }
    }

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

/// A name in scope: where it came from, its default, and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub kind: LocalKind,
    pub default: Option<String>,
    /// The whole declaration, e.g. `freq = 440`.
    pub range: Range<u32>,
    /// Just the name. What goto-definition selects, and what a rename would
    /// rewrite — the same split `sclang_index::Location` makes.
    pub name_range: Range<u32>,
}

/// The pseudo-variables sclang resolves during compilation.
///
/// `PyrLexer.cpp` returns these as plain identifiers — there is no token kind
/// for them — so nothing else in this crate would ever mention them, and a
/// user typing `thi` should still be offered them.
pub const PSEUDO_VARIABLES: &[&str] = &[
    "this",
    "super",
    "thisProcess",
    "thisThread",
    "thisMethod",
    "thisFunction",
    "thisFunctionDef",
];

/// Whether a node opens a scope — somewhere names can be declared.
///
/// `cmdlinecode` lets a script declare variables in a top-level `( … )` block,
/// or bare at the top of the file, and both are scopes like any function body.
/// Without them, the `var`s in the block someone is actually working in are
/// invisible to completion, hover and goto.
pub fn is_scope(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::MethodDef
            | SyntaxKind::FunctionBlock
            | SyntaxKind::ParenExpr
            | SyntaxKind::SourceFile
            | SyntaxKind::ClassDef
            | SyntaxKind::ClassExtension
    )
}

/// The names one scope node declares, in source order.
///
/// [`locals_at`] walks outwards from a point and accumulates these; a walk
/// going the other way — down the tree, colouring as it goes — needs the same
/// collection one node at a time.
pub fn declared_in(node: &SyntaxNode, source: &str) -> Vec<Local> {
    let mut out = Vec::new();
    match node.kind {
        SyntaxKind::MethodDef
        | SyntaxKind::FunctionBlock
        | SyntaxKind::ParenExpr
        | SyntaxKind::SourceFile => collect_declarations(node, source, &mut out),
        SyntaxKind::ClassDef | SyntaxKind::ClassExtension => {
            collect_class_slots(node, source, &mut out)
        }
        _ => {}
    }
    out
}

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
        for local in declared_in(node, source) {
            push_unique(&mut out, local);
        }
    }

    for name in PSEUDO_VARIABLES {
        push_unique(
            &mut out,
            Local {
                name: (*name).to_string(),
                kind: LocalKind::Variable,
                default: None,
                // Bound by the compiler, not written down anywhere, so there
                // is nowhere for goto-definition to go.
                range: 0..0,
                name_range: 0..0,
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
                        range: def.start..def.end,
                        name_range: name.start..name.end,
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
                        range: slot.start..slot.end,
                        name_range: name.start..name.end,
                    },
                );
            }
        }
    }
}

/// The source text of a declaration's default value, if it has one.
///
/// The `=` is optional in the `| … |` argument form — `|range -1|` and
/// `|overwrite(true)|` both declare one — and the parenthesised spelling is
/// not a node of its own, so the default runs from whatever follows the name
/// to the end of the declaration rather than being one child. `sclang-index`
/// reads it the same way, and the two have to agree: they are what hover and
/// signature help say about the same parameter.
fn default_of(def: &SyntaxNode, source: &str) -> Option<String> {
    let name = def.token_of(SyntaxKind::Ident)?;
    let start = def
        .children
        .iter()
        .find(|c| c.range().0 >= name.end && !c.kind().is_trivia() && c.kind() != SyntaxKind::Eq)?
        .range()
        .0;
    (start < def.end).then(|| source[start as usize..def.end as usize].trim().to_string())
}

/// Keep the first sighting of a name: the innermost declaration shadows.
fn push_unique(out: &mut Vec<Local>, local: Local) {
    if !out.iter().any(|l| l.name == local.name) {
        out.push(local);
    }
}

/// A slot visible inside a class's methods, and the class that declares it.
///
/// The declaring class is the whole point: it is usually not the one being
/// looked at, and saying which one it is turns "some variable" into a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot<'a> {
    /// The class the slot is written on, which may be an ancestor.
    pub owner: &'a str,
    pub var: &'a Var,
}

impl Slot<'_> {
    pub fn kind(&self) -> LocalKind {
        LocalKind::of_slot(self.var.kind)
    }
}

/// Every slot visible inside `owner`'s methods: its own, and every one it
/// inherits.
///
/// SuperCollider's class slots are not lexical. `Pgate` reads `pattern`,
/// declared two classes above it on `FilterPattern` and quite possibly in
/// another file, so [`locals_at`] — which walks the tree of one buffer — never
/// sees it. This is the half that has to ask the index.
///
/// Nearest class first. A subclass cannot redeclare an inherited slot, so that
/// is ordering rather than shadowing, but it is what decides which class gets
/// named.
pub fn class_slots<'a>(index: &'a SymbolIndex, owner: &str) -> Vec<Slot<'a>> {
    let mut out: Vec<Slot<'a>> = Vec::new();
    for class in index.superclass_chain(owner) {
        for var in &class.vars {
            if !out.iter().any(|s| s.var.name == var.name) {
                out.push(Slot {
                    owner: &class.name,
                    var,
                });
            }
        }
    }
    out
}

/// One slot by name, if `owner` declares or inherits it.
pub fn class_slot<'a>(index: &'a SymbolIndex, owner: &str, name: &str) -> Option<Slot<'a>> {
    index.superclass_chain(owner).into_iter().find_map(|class| {
        class.vars.iter().find(|v| v.name == name).map(|var| Slot {
            owner: &class.name,
            var,
        })
    })
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
    fn a_default_written_without_an_equals_is_still_a_default() {
        // `slotdef : name optequal slotliteral` — the `=` is optional, and the
        // parenthesised form has no `=` at all. The index reads both, so hover
        // has to as well or it contradicts signature help about the same
        // parameter.
        let source = "T { m { |range -1, overwrite(true)| ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        let found = locals(source, offset);
        let default = |n: &str| {
            found
                .iter()
                .find(|l| l.name == n)
                .and_then(|l| l.default.clone())
        };
        assert_eq!(default("range").as_deref(), Some("-1"));
        assert_eq!(default("overwrite").as_deref(), Some("(true)"));
    }

    #[test]
    fn a_declaration_with_no_value_has_no_default() {
        let source = "T { m { var a; |b| ^x } }";
        let offset = source.find("^x").unwrap() as u32 + 1;
        assert!(locals(source, offset)
            .iter()
            .filter(|l| l.name == "a" || l.name == "b")
            .all(|l| l.default.is_none()));
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
