//! A symbol index for SuperCollider, built over `sclang-syntax` trees.
//!
//! This is what completion, goto-definition and hover are served from. It is
//! built by parsing source, not by asking a running sclang — so it works on a
//! class library that does not compile, which is the state an editor spends
//! most of its time in.
//!
//! The index is per-file, so one file can be re-indexed on a keystroke without
//! rebuilding the world.
//!
//! ```
//! use sclang_index::{MethodKind, SymbolIndex};
//! use std::path::Path;
//!
//! let mut index = SymbolIndex::default();
//! index.index_file(Path::new("Foo.sc"), "Foo : Bar { *new { |a| ^super.new } }");
//!
//! assert_eq!(index.class("Foo").unwrap().superclass.as_deref(), Some("Bar"));
//! assert_eq!(
//!     index.method("Foo", "new", MethodKind::Class).unwrap().signature(),
//!     "*new(a)"
//! );
//! ```

mod build;
mod symbols;

pub use build::{symbols_of, FileSymbols};
pub use symbols::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Identifies a method uniquely: owning class, name, and side.
pub type MethodKey = (String, String, MethodKind);

/// What one file contributed, so it can be withdrawn when the file changes.
#[derive(Debug, Default)]
struct FileEntry {
    classes: Vec<String>,
    methods: Vec<MethodKey>,
}

/// An index of classes and methods.
#[derive(Debug, Default)]
pub struct SymbolIndex {
    classes: BTreeMap<String, Class>,
    methods: BTreeMap<MethodKey, Method>,
    /// Method name to the classes defining it. This is what makes completion
    /// after a `.` possible without types: in a dynamically typed language the
    /// honest answer is "every class that defines this".
    by_name: BTreeMap<String, BTreeSet<(String, MethodKind)>>,
    /// Direct subclasses, for walking the hierarchy downward.
    subclasses: BTreeMap<String, BTreeSet<String>>,
    by_file: BTreeMap<PathBuf, FileEntry>,
}

impl SymbolIndex {
    /// Index one file, replacing anything previously indexed from it.
    pub fn index_file(&mut self, path: &Path, source: &str) {
        self.remove_file(path);
        let syms = symbols_of(path, source);
        let mut entry = FileEntry::default();

        for class in syms.classes {
            entry.classes.push(class.name.clone());
            if let Some(sup) = &class.superclass {
                self.subclasses
                    .entry(sup.clone())
                    .or_default()
                    .insert(class.name.clone());
            }
            self.classes.insert(class.name.clone(), class);
        }

        for method in syms.methods {
            let key = (method.owner.clone(), method.name.clone(), method.kind);
            self.by_name
                .entry(method.name.clone())
                .or_default()
                .insert((method.owner.clone(), method.kind));
            entry.methods.push(key.clone());
            self.methods.insert(key, method);
        }

        self.by_file.insert(path.to_path_buf(), entry);
    }

    /// Withdraw everything a file contributed. Safe for a file never indexed.
    pub fn remove_file(&mut self, path: &Path) {
        let Some(entry) = self.by_file.remove(path) else {
            return;
        };
        for name in entry.classes {
            if let Some(class) = self.classes.remove(&name) {
                if let Some(sup) = class.superclass {
                    if let Some(set) = self.subclasses.get_mut(&sup) {
                        set.remove(&name);
                    }
                }
            }
        }
        for key in entry.methods {
            self.methods.remove(&key);
            if let Some(set) = self.by_name.get_mut(&key.1) {
                set.remove(&(key.0.clone(), key.2));
                if set.is_empty() {
                    self.by_name.remove(&key.1);
                }
            }
        }
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    pub fn method_count(&self) -> usize {
        self.methods.len()
    }

    pub fn class(&self, name: &str) -> Option<&Class> {
        self.classes.get(name)
    }

    pub fn classes(&self) -> impl Iterator<Item = &Class> {
        self.classes.values()
    }

    pub fn methods(&self) -> impl Iterator<Item = &Method> {
        self.methods.values()
    }

    pub fn method(&self, owner: &str, name: &str, kind: MethodKind) -> Option<&Method> {
        self.methods
            .get(&(owner.to_string(), name.to_string(), kind))
    }

    /// Every method declared directly on a class, both sides.
    pub fn methods_on<'a>(&'a self, owner: &str) -> impl Iterator<Item = &'a Method> + 'a {
        let owner = owner.to_string();
        self.methods
            .range((owner.clone(), String::new(), MethodKind::Instance)..)
            .take_while(move |((o, _, _), _)| *o == owner)
            .map(|(_, m)| m)
    }

    /// Classes from `name` up to the root, `name` first.
    ///
    /// Stops at a class that is not indexed, so a partial index yields a
    /// partial chain rather than nothing.
    pub fn superclass_chain(&self, name: &str) -> Vec<&Class> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = Some(name.to_string());
        while let Some(n) = current {
            // Cycles are impossible in valid code but trivial to type in an
            // editor, so guard rather than hang.
            if !seen.insert(n.clone()) {
                break;
            }
            let Some(class) = self.classes.get(&n) else {
                break;
            };
            out.push(class);
            current = class.superclass.clone();
        }
        out
    }

    /// Every method callable on `name`, inherited ones included. A subclass
    /// override shadows the superclass definition.
    pub fn methods_visible_on(&self, name: &str, kind: MethodKind) -> Vec<&Method> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for class in self.superclass_chain(name) {
            for m in self.methods_on(&class.name) {
                if m.kind == kind && seen.insert(m.name.clone()) {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Every class defining a method of this name.
    ///
    /// Without types this is the honest answer for completion after a `.`:
    /// any of these could be the receiver.
    pub fn implementors(&self, method_name: &str) -> Vec<&Method> {
        let Some(set) = self.by_name.get(method_name) else {
            return Vec::new();
        };
        set.iter()
            .filter_map(|(owner, kind)| {
                self.methods
                    .get(&(owner.clone(), method_name.to_string(), *kind))
            })
            .collect()
    }

    /// Direct subclasses of a class.
    pub fn subclasses(&self, name: &str) -> Vec<&Class> {
        self.subclasses
            .get(name)
            .map(|set| set.iter().filter_map(|n| self.classes.get(n)).collect())
            .unwrap_or_default()
    }

    /// Class names beginning with `prefix`, for completing a class name.
    pub fn classes_with_prefix(&self, prefix: &str) -> Vec<&Class> {
        self.classes
            .range(prefix.to_string()..)
            .take_while(|(name, _)| name.starts_with(prefix))
            .map(|(_, c)| c)
            .collect()
    }

    /// Distinct method names beginning with `prefix`, for completing a
    /// selector when the receiver is unknown.
    pub fn method_names_with_prefix(&self, prefix: &str) -> Vec<&str> {
        self.by_name
            .range(prefix.to_string()..)
            .take_while(|(name, _)| name.starts_with(prefix))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}
