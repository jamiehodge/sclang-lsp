//! Rename.
//!
//! Find-references may be generous, because a wrong entry in a list costs the
//! reader a glance. Rename rewrites the file, so it is held to a different
//! standard: it runs only where the set of occurrences is *provably* complete,
//! and refuses with an explanation everywhere else.
//!
//! That leaves two cases:
//!
//! * **Function locals** — arguments and `var`s inside a body. They cannot be
//!   referred to from outside the function, so the enclosing scope bounds the
//!   edit exactly.
//! * **Class names** — only class names lex as `ClassName`, so every reference
//!   is found and nothing else is touched.
//!
//! What it refuses, and why, is the substance of this module. Each refusal
//! below is a case where a plausible-looking rename would quietly break code.

use crate::analysis::{point_at, Bias, Point};
use crate::documents::Document;
use crate::features::find_references::local_references;
use crate::locations::Resolver;
use crate::references::{OccurrenceKind, ReferenceIndex};
use crate::scope::{locals_at, LocalKind};
use lsp_types::{PrepareRenameResponse, TextEdit, Url, WorkspaceEdit};
use sclang_index::SymbolIndex;
use sclang_syntax::{tokenize, SyntaxKind};
use std::collections::HashMap;

/// What can be renamed at a point, or why it cannot.
enum Target {
    /// A binding confined to one function body.
    Local {
        name: String,
    },
    /// A class, and every mention of it in the workspace.
    Class {
        name: String,
    },
    Refused(String),
}

fn target_at(doc: &Document, index: &SymbolIndex, offset: u32) -> Target {
    let root = &doc.parse().root;
    let source = &doc.text;

    match point_at(root, source, offset, Bias::Inside) {
        Point::ClassName(name) => {
            if index.class(&name).is_none() {
                return Target::Refused(format!(
                    "`{name}` is not a class this server has indexed, so its uses cannot be found"
                ));
            }
            Target::Class { name }
        }

        Point::Local { name } => {
            let Some(local) = locals_at(root, source, offset)
                .into_iter()
                .find(|l| l.name == name)
            else {
                return Target::Refused(format!("`{name}` is not bound in any enclosing scope"));
            };

            match local.kind {
                LocalKind::Argument | LocalKind::Variable => {
                    if local.name_range.end == local.name_range.start {
                        return Target::Refused(format!(
                            "`{name}` is bound by the compiler and cannot be renamed"
                        ));
                    }
                    Target::Local { name }
                }
                // A `var <count` generates the methods `count` and `count_`,
                // so renaming the variable renames methods that may be called
                // from anywhere. And subclasses inherit instance variables,
                // so even without accessors the uses are not confined to this
                // class.
                LocalKind::InstanceVar => Target::Refused(format!(
                    "`{name}` is an instance variable. Renaming it would rename any generated \
                     accessor with it, and subclasses can refer to it, so the uses are not \
                     confined to this file."
                )),
                LocalKind::ClassVar => Target::Refused(format!(
                    "`{name}` is a classvar. Subclasses inherit it and generated accessors \
                     travel with it, so its uses are not confined to this file."
                )),
                LocalKind::Constant => Target::Refused(format!(
                    "`{name}` is a const, which is visible to subclasses; its uses are not \
                     confined to this file."
                )),
            }
        }

        // The heart of it. `.play` is dispatched at run time, so the `play` on
        // one class and the `play` on another are the same token and a
        // different method. Renaming every `.play` would break every class
        // that was not meant; renaming only some would break this one.
        Point::Selector { name, .. } | Point::MethodName { name, .. } => Target::Refused(format!(
            "`{name}` is a method. SuperCollider dispatches at run time, so there is no way to \
             tell which `.{name}` sends refer to this definition — renaming them all would break \
             unrelated classes. Use find-references and edit deliberately."
        )),

        Point::Nothing => Target::Refused("there is nothing renameable here".to_string()),
    }
}

/// Whether a rename can start here, and over what text.
pub fn prepare_rename(
    doc: &Document,
    index: &SymbolIndex,
    offset: u32,
    enc: crate::line_index::PositionEncoding,
) -> Result<PrepareRenameResponse, String> {
    match target_at(doc, index, offset) {
        Target::Refused(why) => Err(why),
        Target::Local { name } | Target::Class { name } => {
            let token = crate::analysis::token_at(&doc.parse().root, offset, Bias::Inside)
                .ok_or_else(|| "there is nothing renameable here".to_string())?;
            Ok(PrepareRenameResponse::RangeWithPlaceholder {
                range: doc
                    .line_index
                    .range(&doc.text, token.token.start..token.token.end, enc),
                placeholder: name,
            })
        }
    }
}

pub fn rename(
    uri: &Url,
    doc: &Document,
    index: &SymbolIndex,
    refs: &ReferenceIndex,
    offset: u32,
    new_name: &str,
    resolver: &Resolver<'_>,
) -> Result<WorkspaceEdit, String> {
    match target_at(doc, index, offset) {
        Target::Refused(why) => Err(why),

        Target::Local { name } => {
            check_name(new_name, false)?;
            let ranges = local_references(doc, offset, &name, true);
            if ranges.is_empty() {
                return Err(format!("found no uses of `{name}` to rename"));
            }
            let edits = ranges
                .into_iter()
                .map(|r| TextEdit {
                    range: doc.line_index.range(&doc.text, r, resolver.encoding()),
                    new_text: new_name.to_string(),
                })
                .collect();
            Ok(WorkspaceEdit {
                changes: Some(HashMap::from([(uri.clone(), edits)])),
                ..Default::default()
            })
        }

        Target::Class { name } => {
            check_name(new_name, true)?;
            if index.class(new_name).is_some() {
                return Err(format!("a class named `{new_name}` already exists"));
            }

            let found = refs.find(&name, |kind| {
                matches!(
                    kind,
                    OccurrenceKind::Class | OccurrenceKind::ClassDefinition
                )
            });
            let locations = resolver.at_many(found.into_iter().map(|(p, o)| (p, o.range.clone())));
            if locations.is_empty() {
                return Err(format!("found no uses of `{name}` to rename"));
            }

            let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
            for location in locations {
                changes.entry(location.uri).or_default().push(TextEdit {
                    range: location.range,
                    new_text: new_name.to_string(),
                });
            }
            Ok(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            })
        }
    }
}

/// Reject a new name the language would not lex the same way.
///
/// Case is not style in SuperCollider: an initial capital makes a token a
/// `ClassName` and anything else an `Ident`. Renaming a class to `sinOsc`
/// would not produce a differently-named class, it would produce a syntax
/// error at every use.
///
/// The test is to lex the name and see what comes out, rather than to restate
/// the lexer's rules here. A character-by-character version was both too loose
/// and too tight: it accepted `var`, `nil` and `inf`, which are keywords and a
/// float literal, so renaming a local to one of them wrote `var var = 1;` into
/// the file — a silent syntax error, which is the one thing this module exists
/// to avoid. And it rejected a non-ASCII letter, which sclang's own character
/// table puts in the identifier class.
fn check_name(name: &str, class: bool) -> Result<(), String> {
    let wanted = if class {
        SyntaxKind::ClassName
    } else {
        SyntaxKind::Ident
    };

    let tokens = tokenize(name);
    let [token] = tokens.as_slice() else {
        return Err(if tokens.is_empty() {
            "the new name is empty".to_string()
        } else {
            format!("`{name}` is not a valid identifier: it is more than one token")
        });
    };
    if token.kind == wanted {
        return Ok(());
    }

    Err(match (class, token.kind) {
        (true, SyntaxKind::Ident) => {
            format!("`{name}` cannot name a class: class names must begin with a capital letter")
        }
        (false, SyntaxKind::ClassName) => format!(
            "`{name}` cannot name a variable: an initial capital would make it a class name"
        ),
        // `var`, `while`, `nil`, `pi` — and `inf`, which sclang lexes as a
        // float rather than as a keyword.
        (
            _,
            SyntaxKind::VarKw
            | SyntaxKind::ArgKw
            | SyntaxKind::ClassvarKw
            | SyntaxKind::ConstKw
            | SyntaxKind::WhileKw
            | SyntaxKind::TrueKw
            | SyntaxKind::FalseKw
            | SyntaxKind::NilKw
            | SyntaxKind::PiKw
            | SyntaxKind::Float,
        ) => format!("`{name}` is reserved: SuperCollider reads it as a keyword or a literal"),
        _ => format!("`{name}` is not a valid identifier"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::DocumentStore;

    /// Renaming a class rewrites files across the workspace, and converting
    /// those byte ranges to line and column means reading them — so the
    /// fixture is a real file rather than a path that merely looks like one.
    fn try_rename(source: &str, cursor: &str, new_name: &str) -> Result<WorkspaceEdit, String> {
        let dir = std::env::temp_dir().join(format!(
            "sclang-lsp-rename-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.sc");
        std::fs::write(&file, source).unwrap();

        let mut index = SymbolIndex::default();
        let mut refs = ReferenceIndex::default();
        index.index_file(&file, source);
        refs.index_file(&file, source);

        let doc = Document::new(source.to_string(), 1);
        let docs = DocumentStore::default();
        let resolver = Resolver::new(&docs, crate::line_index::PositionEncoding::Utf16);
        let uri = Url::from_file_path(&file).unwrap();
        let offset = source.find(cursor).expect("cursor") as u32;

        let result = rename(&uri, &doc, &index, &refs, offset, new_name, &resolver);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn edit_count(edit: &WorkspaceEdit) -> usize {
        edit.changes
            .as_ref()
            .unwrap()
            .values()
            .map(|v| v.len())
            .sum()
    }

    #[test]
    fn renames_a_local_everywhere_in_its_scope() {
        let edit = try_rename("{ var foo = 1; foo + foo }", "foo = 1", "bar").unwrap();
        // The declaration and both uses.
        assert_eq!(edit_count(&edit), 3);
    }

    #[test]
    fn renaming_a_local_leaves_an_inner_shadow_alone() {
        let source = "{ |v| { |v| v } ; v }";
        // The `v` of the outer `|v|`, not the pipe before it.
        let edit = try_rename(source, "v| { |v|", "w").unwrap();
        // Only the outer binding and its one use, not the inner pair.
        assert_eq!(edit_count(&edit), 2);
    }

    #[test]
    fn renames_a_class_and_its_uses() {
        let edit = try_rename(
            "Foo : Object { }\nBar : Foo { m { ^Foo.new } }",
            "Foo :",
            "Baz",
        )
        .unwrap();
        // The definition, the superclass mention, and the receiver.
        assert_eq!(edit_count(&edit), 3);
    }

    #[test]
    fn refuses_to_rename_a_method() {
        let error = try_rename("A { play { ^1 } }", "play {", "start").unwrap_err();
        assert!(error.contains("dispatches at run time"), "{error}");
    }

    #[test]
    fn refuses_to_rename_a_selector() {
        let error = try_rename("A { m { ^x.play } }", "play", "start").unwrap_err();
        assert!(error.contains("dispatches at run time"), "{error}");
    }

    #[test]
    fn refuses_to_rename_an_instance_variable() {
        let error = try_rename("A { var <count; m { ^count } }", "count;", "total").unwrap_err();
        assert!(error.contains("accessor"), "{error}");
    }

    #[test]
    fn refuses_a_class_name_that_would_not_lex_as_one() {
        let error = try_rename("Foo : Object { }", "Foo :", "foo").unwrap_err();
        assert!(error.contains("capital letter"), "{error}");
    }

    #[test]
    fn refuses_a_local_name_that_would_lex_as_a_class() {
        let error = try_rename("{ var foo = 1; foo }", "foo = 1", "Foo").unwrap_err();
        assert!(error.contains("class name"), "{error}");
    }

    #[test]
    fn refuses_to_collide_with_an_existing_class() {
        let error = try_rename("Foo : Object { }\nBar : Object { }", "Foo :", "Bar").unwrap_err();
        assert!(error.contains("already exists"), "{error}");
    }

    #[test]
    fn refuses_a_name_with_punctuation() {
        let error = try_rename("{ var foo = 1; foo }", "foo = 1", "ba-r").unwrap_err();
        assert!(error.contains("valid identifier"), "{error}");
    }

    #[test]
    fn refuses_a_reserved_word() {
        // These all pass a letters-and-digits test and none of them is a name:
        // `var var = 1;` is a syntax error, and the rename that wrote it would
        // have been silent. `inf` is here because sclang lexes it as a float
        // rather than as a keyword, so a list of keywords would miss it.
        for name in [
            "var", "arg", "classvar", "const", "while", "nil", "true", "false", "pi", "inf",
        ] {
            let error = try_rename("{ var foo = 1; foo }", "foo = 1", name).unwrap_err();
            assert!(error.contains("reserved"), "{name}: {error}");
        }
    }

    #[test]
    fn refuses_a_name_that_is_more_than_one_token() {
        for name in ["two names", "foo:", "foo;", ""] {
            let error = try_rename("{ var foo = 1; foo }", "foo = 1", name).unwrap_err();
            assert!(!error.is_empty(), "{name:?} was accepted");
        }
    }

    #[test]
    fn accepts_a_non_ascii_letter() {
        // sclang's character table puts a byte above ASCII in the identifier
        // class — `var ±x = 1;` compiles — so a check stricter than the lexer
        // would refuse a rename the language allows.
        assert!(try_rename("{ var foo = 1; foo }", "foo = 1", "café").is_ok());
    }
}
