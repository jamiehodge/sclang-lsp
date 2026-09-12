//! Where every name is written, across the workspace.
//!
//! The symbol index records where things are *defined*. Find-references and
//! rename need where they are *used*, which is a different and much larger
//! set, so it lives here rather than being forced into `sclang-index`.
//!
//! Occurrences are kept per file so one file can be re-walked on a keystroke,
//! mirroring how the symbol index is maintained. Queries scan every file's
//! list: with a few hundred thousand occurrences that costs single-digit
//! milliseconds, and it removes a whole category of stale-index bug in
//! exchange.

use sclang_syntax::{parse, Child, SyntaxKind, SyntaxNode};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// What a name is doing where it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccurrenceKind {
    /// The name of a class where it is defined: `Foo` in `Foo : Bar { }`.
    ClassDefinition,
    /// A class name used anywhere else: a receiver, a superclass, a `+ Foo`
    /// extension header.
    Class,
    /// The name in a method definition: `bar` in `bar { ... }`.
    MethodDefinition,
    /// A selector being sent: `bar` in `foo.bar` or in `bar(foo)`.
    MethodReference,
}

/// One name, written once, somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub name: String,
    pub kind: OccurrenceKind,
    pub range: Range<u32>,
}

#[derive(Debug, Default)]
pub struct ReferenceIndex {
    by_file: BTreeMap<PathBuf, Vec<Occurrence>>,
}

impl ReferenceIndex {
    /// Walk one file, replacing anything recorded from it before.
    pub fn index_file(&mut self, path: &Path, source: &str) {
        let parsed = parse(source);
        self.index_parsed(path, source, &parsed.root);
    }

    /// The same, for a file whose tree is already in hand.
    ///
    /// Open documents keep a parse, and re-parsing it on every keystroke to
    /// rebuild this would double the cost of typing for no gain.
    pub fn index_parsed(&mut self, path: &Path, source: &str, root: &SyntaxNode) {
        let mut out = Vec::new();
        collect(root, source, &mut out);
        self.by_file.insert(path.to_path_buf(), out);
    }

    pub fn remove_file(&mut self, path: &Path) {
        self.by_file.remove(path);
    }

    pub fn occurrence_count(&self) -> usize {
        self.by_file.values().map(|v| v.len()).sum()
    }

    /// Every occurrence of `name` whose kind the predicate accepts.
    ///
    /// Collected rather than lazy: the result is walked once by the caller and
    /// a borrowed iterator here buys nothing but lifetime trouble.
    pub fn find(
        &self,
        name: &str,
        accept: impl Fn(OccurrenceKind) -> bool,
    ) -> Vec<(&Path, &Occurrence)> {
        self.by_file
            .iter()
            .flat_map(|(path, occurrences)| occurrences.iter().map(move |o| (path.as_path(), o)))
            .filter(|(_, o)| o.name == name && accept(o.kind))
            .collect()
    }
}

/// Record every class name and selector in a tree.
fn collect(node: &SyntaxNode, source: &str, out: &mut Vec<Occurrence>) {
    // The one class name that is a definition rather than a use. A `+ Foo`
    // extension header is not one: it names a class defined elsewhere.
    let defines = (node.kind == SyntaxKind::ClassDef)
        .then(|| node.token_of(SyntaxKind::ClassName).map(|t| t.start))
        .flatten();

    for child in &node.children {
        match child {
            Child::Token(token) => {
                if token.kind == SyntaxKind::ClassName {
                    out.push(Occurrence {
                        name: token.text(source).to_string(),
                        kind: if Some(token.start) == defines {
                            OccurrenceKind::ClassDefinition
                        } else {
                            OccurrenceKind::Class
                        },
                        range: token.start..token.end,
                    });
                }
            }
            Child::Node(inner) => collect(inner, source, out),
        }
    }

    // Selectors and method names are identifiers, which are only
    // distinguishable by what encloses them.
    match node.kind {
        SyntaxKind::MethodDef => {
            if let Some(name) = node.token_of(SyntaxKind::Ident) {
                out.push(Occurrence {
                    name: name.text(source).to_string(),
                    kind: OccurrenceKind::MethodDefinition,
                    range: name.start..name.end,
                });
            }
        }
        SyntaxKind::MethodCall => {
            // Every `.selector` in the chain, not just the first: `a.b.c`
            // sends both `b` and `c`.
            for dot in node.child_tokens().filter(|t| t.kind == SyntaxKind::Dot) {
                if let Some(selector) = node
                    .child_tokens()
                    .find(|t| t.kind == SyntaxKind::Ident && t.start >= dot.end)
                {
                    out.push(Occurrence {
                        name: selector.text(source).to_string(),
                        kind: OccurrenceKind::MethodReference,
                        range: selector.start..selector.end,
                    });
                }
            }
        }
        SyntaxKind::CallExpr => {
            // `foo(a, b)` is `a.foo(b)`, so the head is a selector too.
            if let Some(head) = node.child_nodes().next() {
                if head.kind == SyntaxKind::NameRef {
                    if let Some(name) = head.token_of(SyntaxKind::Ident) {
                        out.push(Occurrence {
                            name: name.text(source).to_string(),
                            kind: OccurrenceKind::MethodReference,
                            range: name.start..name.end,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(source: &str) -> ReferenceIndex {
        let mut index = ReferenceIndex::default();
        index.index_file(Path::new("a.sc"), source);
        index
    }

    fn found(index: &ReferenceIndex, name: &str) -> Vec<OccurrenceKind> {
        let mut kinds: Vec<_> = index
            .find(name, |_| true)
            .into_iter()
            .map(|(_, o)| o.kind)
            .collect();
        kinds.sort_by_key(|k| format!("{k:?}"));
        kinds
    }

    #[test]
    fn records_class_names_wherever_they_appear() {
        let index = index("Foo : Bar { baz { ^Qux.new } }");
        assert_eq!(found(&index, "Foo"), vec![OccurrenceKind::ClassDefinition]);
        assert_eq!(found(&index, "Bar"), vec![OccurrenceKind::Class]);
        assert_eq!(found(&index, "Qux"), vec![OccurrenceKind::Class]);
    }

    #[test]
    fn an_extension_header_names_a_class_defined_elsewhere() {
        let index = index("+ Foo { extra { ^1 } }");
        assert_eq!(found(&index, "Foo"), vec![OccurrenceKind::Class]);
    }

    #[test]
    fn separates_definitions_from_sends() {
        let index = index("Foo { play { ^1 } } Bar { run { ^this.play } }");
        assert_eq!(
            found(&index, "play"),
            vec![
                OccurrenceKind::MethodDefinition,
                OccurrenceKind::MethodReference
            ]
        );
    }

    #[test]
    fn a_chain_sends_every_selector() {
        let index = index("x.one.two.three");
        for name in ["one", "two", "three"] {
            assert_eq!(
                found(&index, name),
                vec![OccurrenceKind::MethodReference],
                "{name}"
            );
        }
    }

    #[test]
    fn functional_notation_is_a_send() {
        // `postln(x)` is `x.postln`.
        let index = index("postln(x)");
        assert_eq!(
            found(&index, "postln"),
            vec![OccurrenceKind::MethodReference]
        );
    }

    #[test]
    fn ranges_point_at_the_name() {
        let source = "Foo { bar { ^1 } }";
        let index = index(source);
        let found = index.find("bar", |_| true);
        let (_, occurrence) = found.first().unwrap();
        assert_eq!(
            &source[occurrence.range.start as usize..occurrence.range.end as usize],
            "bar"
        );
    }

    #[test]
    fn a_symbol_is_not_a_selector() {
        // `\\play` and "play" are data, not sends, and must not be rewritten
        // by a rename.
        let index = index("x = \\play; y = \"play\";");
        assert!(found(&index, "play").is_empty());
    }

    #[test]
    fn reindexing_replaces_the_previous_walk() {
        let mut index = ReferenceIndex::default();
        index.index_file(Path::new("a.sc"), "Foo { }");
        index.index_file(Path::new("a.sc"), "Bar { }");
        assert!(found(&index, "Foo").is_empty());
        assert_eq!(found(&index, "Bar"), vec![OccurrenceKind::ClassDefinition]);
    }

    #[test]
    fn removing_a_file_withdraws_its_occurrences() {
        let mut index = index("Foo { }");
        index.remove_file(Path::new("a.sc"));
        assert!(found(&index, "Foo").is_empty());
        assert_eq!(index.occurrence_count(), 0);
    }
}
