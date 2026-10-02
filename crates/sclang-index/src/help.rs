//! Class documentation, read from the `.schelp` files sclang renders its help
//! from.
//!
//! The lookup mirrors `SCDoc.sc`, because that is what decides which text the
//! help browser shows for a method:
//!
//! - A class's page is the document at `Classes/<Name>` inside any
//!   `HelpSource` directory — by path, not by its `class::` line.
//! - `Classes/<Name>.ext.schelp` adds to that page; it is how a quark
//!   documents methods it adds to someone else's class. The page itself is
//!   consulted first, then its additions.
//! - `method:: freq` documents the setter `freq_` as well, as `asGetter` has
//!   it.
//! - `copymethod:: Bar *new` borrows another page's entry, looked up when it
//!   is asked for, so it follows that page when it changes.
//!
//! Only class pages are kept, and of each only what is shown outside the help
//! browser: the summary, the description, and the methods. Guides and
//! examples are read from disk when something asks for a whole page.

use crate::MethodKind;
use sclang_scdoc::{Id, Mode, Node};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// How many `copymethod::` hops to follow. Two pages copying from each other
/// is an error nobody will see until an editor loops on it.
const MAX_COPIES: usize = 8;

/// Documentation from every class page found.
#[derive(Debug, Default)]
pub struct HelpIndex {
    files: BTreeMap<PathBuf, HelpFile>,
    /// Class name to the files documenting it, pages and additions alike.
    by_class: BTreeMap<String, BTreeSet<PathBuf>>,
}

#[derive(Debug)]
struct HelpFile {
    class: String,
    /// An `.ext.schelp`, which adds to a page rather than being one.
    addition: bool,
    summary: Option<String>,
    description: Vec<Node>,
    methods: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    kind: MethodKind,
    names: Vec<String>,
    doc: EntryDoc,
}

#[derive(Debug)]
enum EntryDoc {
    /// The children of a `METHODBODY`.
    Body(Vec<Node>),
    /// `copymethod::`, still to be looked up.
    Copy {
        class: String,
        name: String,
        kind: MethodKind,
    },
}

/// What a class page says about the class itself.
#[derive(Debug, Clone, Copy)]
pub struct ClassHelp<'a> {
    pub summary: Option<&'a str>,
    /// The body of `description::`, for [`sclang_scdoc::markdown::render`].
    pub description: &'a [Node],
    pub file: &'a Path,
}

/// What a class page says about one of its methods.
#[derive(Debug, Clone, Copy)]
pub struct MethodHelp<'a> {
    /// The children of the method's `METHODBODY`: prose and body elements,
    /// then `ARGUMENTS`, `RETURNS` and `DISCUSSION` if it has them.
    pub body: &'a [Node],
    /// The file the text is in, which for a `copymethod::` is the other
    /// class's.
    pub file: &'a Path,
}

impl<'a> MethodHelp<'a> {
    /// The text before the arguments: what the method does.
    pub fn prose(&self) -> Vec<Node> {
        self.body
            .iter()
            .filter(|n| !matches!(n.id, Id::Arguments | Id::Returns | Id::Discussion))
            .cloned()
            .collect()
    }

    /// Each documented argument's name, as written, with its description.
    /// An `argument::` with no name is skipped, since nothing can match it.
    pub fn arguments(&self) -> impl Iterator<Item = (&'a str, &'a [Node])> {
        self.section(Id::Arguments)
            .unwrap_or(&[])
            .iter()
            .filter_map(|a| Some((a.text.as_deref()?, a.children.as_slice())))
    }

    /// The description of the argument named `name`. Matched loosely enough
    /// that `...args` finds `... args` and `args`, which help files write all
    /// three ways.
    pub fn argument(&self, name: &str) -> Option<&'a [Node]> {
        let bare = |s: &str| s.trim().trim_start_matches('.').trim().to_string();
        let name = bare(name);
        self.arguments()
            .find(|(n, _)| bare(n) == name)
            .map(|(_, body)| body)
    }

    pub fn returns(&self) -> Option<&'a [Node]> {
        self.section(Id::Returns)
    }

    pub fn discussion(&self) -> Option<&'a [Node]> {
        self.section(Id::Discussion)
    }

    fn section(&self, id: Id) -> Option<&'a [Node]> {
        self.body
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.children.as_slice())
    }
}

/// Why a help file was not indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// Not a class page: a guide, a tutorial, a reference.
    NotAClass,
    /// It does not parse, so sclang would render nothing for it either.
    Invalid(sclang_scdoc::Error),
}

impl HelpIndex {
    /// Index the help file at `path`, inside the `HelpSource` directory
    /// `root`, replacing anything indexed from it before.
    pub fn index_file(&mut self, root: &Path, path: &Path, source: &[u8]) -> Result<(), Skipped> {
        self.remove_file(path);
        let (class, addition) = class_of(root, path).ok_or(Skipped::NotAClass)?;
        let mode = if addition { Mode::Partial } else { Mode::Full };
        let tree = sclang_scdoc::parse(source, mode).map_err(Skipped::Invalid)?;
        let file = read(class.clone(), addition, tree);
        self.by_class
            .entry(class)
            .or_default()
            .insert(path.to_path_buf());
        self.files.insert(path.to_path_buf(), file);
        Ok(())
    }

    /// Withdraw a file. Safe for one never indexed.
    pub fn remove_file(&mut self, path: &Path) {
        let Some(file) = self.files.remove(path) else {
            return;
        };
        if let Some(set) = self.by_class.get_mut(&file.class) {
            set.remove(path);
            if set.is_empty() {
                self.by_class.remove(&file.class);
            }
        }
    }

    /// How many class pages and additions are indexed.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The class's own page.
    pub fn class(&self, name: &str) -> Option<ClassHelp<'_>> {
        let (path, file) = self.pages(name).next()?;
        Some(ClassHelp {
            summary: file.summary.as_deref(),
            description: &file.description,
            file: path,
        })
    }

    /// What `owner`'s page, or one of its additions, says about a method.
    ///
    /// Only `owner`'s documentation is consulted. An override documented
    /// nowhere is not described by its superclass's text: the two often
    /// differ in exactly the way the override exists for.
    pub fn method(&self, owner: &str, name: &str, kind: MethodKind) -> Option<MethodHelp<'_>> {
        self.method_at_depth(owner, name, kind, 0)
    }

    fn method_at_depth(
        &self,
        owner: &str,
        name: &str,
        kind: MethodKind,
        depth: usize,
    ) -> Option<MethodHelp<'_>> {
        let name = getter(name);
        let (path, entry) = self.pages(owner).find_map(|(path, file)| {
            file.methods
                .iter()
                .find(|e| e.kind == kind && e.names.iter().any(|n| n == name))
                .map(|e| (path, e))
        })?;
        match &entry.doc {
            EntryDoc::Body(body) => Some(MethodHelp { body, file: path }),
            EntryDoc::Copy { class, name, kind } if depth < MAX_COPIES => {
                self.method_at_depth(class, name, *kind, depth + 1)
            }
            EntryDoc::Copy { .. } => None,
        }
    }

    /// A class's page, then its additions. Should two `HelpSource`
    /// directories both have the page, the first by path is used.
    fn pages(&self, class: &str) -> impl Iterator<Item = (&Path, &HelpFile)> {
        let files: Vec<_> = self
            .by_class
            .get(class)
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p.as_path(), self.files.get(p)?)))
            .collect();
        let (pages, additions): (Vec<_>, Vec<_>) =
            files.into_iter().partition(|(_, f)| !f.addition);
        pages.into_iter().take(1).chain(additions)
    }
}

/// The class a file documents, if it is a class page or an addition to one:
/// `Classes/<Name>.schelp` or `Classes/<Name>.ext.schelp` under the root.
pub fn class_of(root: &Path, path: &Path) -> Option<(String, bool)> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = rel.components();
    if parts.next()?.as_os_str() != "Classes" {
        return None;
    }
    let file = parts.next()?.as_os_str().to_str()?;
    if parts.next().is_some() {
        return None;
    }
    let (name, addition) = match file.strip_suffix(".ext.schelp") {
        Some(name) => (name, true),
        None => (file.strip_suffix(".schelp")?, false),
    };
    let is_class_name = name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_class_name.then(|| (name.to_string(), addition))
}

/// Keep what is shown outside the help browser, and let the rest go.
fn read(class: String, addition: bool, tree: Node) -> HelpFile {
    let mut summary = None;
    let mut body = None;
    match tree.id {
        // A page: a header, then the body.
        Id::Document => {
            for child in tree.children {
                match child.id {
                    Id::Header => {
                        summary = child
                            .children
                            .into_iter()
                            .find(|h| h.id == Id::Summary)
                            .and_then(|s| s.text);
                    }
                    Id::Body => body = Some(child),
                    _ => {}
                }
            }
        }
        // An addition, which has no header.
        Id::Body => body = Some(tree),
        _ => {}
    }

    let mut file = HelpFile {
        class,
        addition,
        summary,
        description: Vec::new(),
        methods: Vec::new(),
    };
    if let Some(body) = body {
        for section in body.children {
            if section.id == Id::Description && file.description.is_empty() {
                // Methods documented inside a description are still methods.
                collect_methods(&section, &mut file.methods);
                file.description = section.children;
            } else {
                collect_methods(&section, &mut file.methods);
            }
        }
    }
    file
}

/// Every class- and instance-method entry under `node`, in document order.
/// The parser already settled which side each is on, from the section it is
/// in, so subsections need no special handling.
fn collect_methods(node: &Node, out: &mut Vec<Entry>) {
    let kind = match node.id {
        Id::CMethod | Id::CCopyMethod => Some(MethodKind::Class),
        Id::IMethod | Id::ICopyMethod => Some(MethodKind::Instance),
        _ => None,
    };
    match (node.id, kind) {
        (Id::CMethod | Id::IMethod, Some(kind)) => {
            let names = node
                .child(Id::MethodNames)
                .map(|n| n.children.iter().filter_map(|s| s.text.clone()).collect())
                .unwrap_or_default();
            let body = node
                .child(Id::MethodBody)
                .map(|b| b.children.clone())
                .unwrap_or_default();
            out.push(Entry {
                kind,
                names,
                doc: EntryDoc::Body(body),
            });
        }
        (Id::CCopyMethod | Id::ICopyMethod, Some(kind)) => {
            if let Some(entry) = copy_entry(node.text.as_deref().unwrap_or(""), kind) {
                out.push(entry);
            }
        }
        _ => {
            for child in &node.children {
                collect_methods(child, out);
            }
        }
    }
}

/// `copymethod:: Bar *new`: the class, then the method with `*` for the
/// class side, `-` for the instance side, or `.` for a page's free-standing
/// method, which no class or instance method can be looked up as.
fn copy_entry(text: &str, kind: MethodKind) -> Option<Entry> {
    // `findRegexp("[^ ,]+")`: the first two runs of anything but spaces and
    // commas.
    let mut words = text.split([' ', ',']).filter(|w| !w.is_empty());
    let class = words.next()?.to_string();
    let target = words.next()?;
    let from = match target.chars().next()? {
        '*' => MethodKind::Class,
        '-' => MethodKind::Instance,
        _ => return None,
    };
    let name = target[1..].to_string();
    Some(Entry {
        kind,
        names: vec![name.clone()],
        doc: EntryDoc::Copy {
            class,
            name,
            kind: from,
        },
    })
}

/// `asGetter`: a setter is documented under its getter's name.
fn getter(name: &str) -> &str {
    match name.strip_suffix('_') {
        Some(getter) if !getter.is_empty() => getter,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(files: &[(&str, &str)]) -> HelpIndex {
        let mut help = HelpIndex::default();
        for (path, source) in files {
            let path = Path::new("/help").join(path);
            let _ = help.index_file(Path::new("/help"), &path, source.as_bytes());
        }
        help
    }

    fn render(nodes: &[Node]) -> String {
        sclang_scdoc::markdown::render(nodes)
    }

    const SIN_OSC: &str = "class:: SinOsc\nsummary:: A sine.\ndescription::\nIt oscillates.\n\
        classmethods::\nmethod:: ar, kr\nMakes one.\nargument:: freq\nIn Hertz.\n\
        argument:: ... args\nThe rest.\nreturns:: A UGen.\n\
        instancemethods::\nmethod:: freq\nThe frequency.\n";

    #[test]
    fn a_class_page() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        let class = help.class("SinOsc").expect("class help");
        assert_eq!(class.summary, Some("A sine."));
        assert_eq!(render(class.description), "It oscillates.");
    }

    #[test]
    fn every_name_of_a_method_entry_finds_it() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        for name in ["ar", "kr"] {
            let m = help.method("SinOsc", name, MethodKind::Class).unwrap();
            assert_eq!(render(&m.prose()), "Makes one.");
            assert_eq!(render(m.argument("freq").unwrap()), "In Hertz.");
            assert_eq!(render(m.returns().unwrap()), "A UGen.");
        }
    }

    #[test]
    fn the_side_matters() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        assert!(help.method("SinOsc", "ar", MethodKind::Instance).is_none());
        let m = help.method("SinOsc", "freq", MethodKind::Instance).unwrap();
        assert_eq!(render(&m.prose()), "The frequency.");
    }

    #[test]
    fn a_setter_reads_its_getter() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        assert!(help
            .method("SinOsc", "freq_", MethodKind::Instance)
            .is_some());
    }

    #[test]
    fn a_rest_argument_matches_however_it_is_written() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        let m = help.method("SinOsc", "ar", MethodKind::Class).unwrap();
        assert!(m.argument("args").is_some());
        assert!(m.argument("...args").is_some());
    }

    #[test]
    fn an_addition_documents_more_methods_but_the_page_wins() {
        let help = index(&[
            ("Classes/SinOsc.schelp", SIN_OSC),
            (
                "Classes/SinOsc.ext.schelp",
                "instancemethods::\nmethod:: wobble\nFrom a quark.\nmethod:: freq\nOverridden?\n",
            ),
        ]);
        let wobble = help
            .method("SinOsc", "wobble", MethodKind::Instance)
            .unwrap();
        assert_eq!(render(&wobble.prose()), "From a quark.");
        let freq = help.method("SinOsc", "freq", MethodKind::Instance).unwrap();
        assert_eq!(render(&freq.prose()), "The frequency.");
        // The addition is not a page, so the class still has its own summary.
        assert_eq!(help.class("SinOsc").unwrap().summary, Some("A sine."));
    }

    #[test]
    fn copymethod_borrows_from_another_page() {
        let help = index(&[
            ("Classes/SinOsc.schelp", SIN_OSC),
            (
                "Classes/FSinOsc.schelp",
                "class:: FSinOsc\nsummary:: Fast.\nclassmethods::\ncopymethod:: SinOsc *ar\n",
            ),
        ]);
        let m = help.method("FSinOsc", "ar", MethodKind::Class).unwrap();
        assert_eq!(render(&m.prose()), "Makes one.");
        assert!(m.file.ends_with("SinOsc.schelp"));
    }

    #[test]
    fn copies_that_go_round_in_circles_end() {
        let help = index(&[
            (
                "Classes/A.schelp",
                "class:: A\nsummary:: a\nclassmethods::\ncopymethod:: B *x\n",
            ),
            (
                "Classes/B.schelp",
                "class:: B\nsummary:: b\nclassmethods::\ncopymethod:: A *x\n",
            ),
        ]);
        assert!(help.method("A", "x", MethodKind::Class).is_none());
    }

    #[test]
    fn an_undocumented_override_says_nothing() {
        let help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        assert!(help.method("FSinOsc", "ar", MethodKind::Class).is_none());
        assert!(help
            .method("SinOsc", "play", MethodKind::Instance)
            .is_none());
    }

    #[test]
    fn only_class_pages_are_indexed() {
        let mut help = HelpIndex::default();
        let root = Path::new("/help");
        let guide = root.join("Guides/Intro.schelp");
        assert_eq!(
            help.index_file(root, &guide, b"title:: Intro\nsummary:: x\n"),
            Err(Skipped::NotAClass)
        );
        let broken = root.join("Classes/Broken.schelp");
        assert!(matches!(
            help.index_file(root, &broken, b"class::\n"),
            Err(Skipped::Invalid(_))
        ));
        assert!(help.is_empty());
    }

    #[test]
    fn reindexing_replaces_and_removing_withdraws() {
        let mut help = index(&[("Classes/SinOsc.schelp", SIN_OSC)]);
        let path = Path::new("/help/Classes/SinOsc.schelp");
        help.index_file(
            Path::new("/help"),
            path,
            b"class:: SinOsc\nsummary:: Changed.\n",
        )
        .unwrap();
        assert_eq!(help.class("SinOsc").unwrap().summary, Some("Changed."));
        assert!(help.method("SinOsc", "ar", MethodKind::Class).is_none());
        help.remove_file(path);
        assert!(help.class("SinOsc").is_none());
    }
}
