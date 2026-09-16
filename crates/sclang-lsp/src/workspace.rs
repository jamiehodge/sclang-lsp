//! Finding SuperCollider source on disk and indexing it.
//!
//! Everything here works by reading files, so it is available before sclang
//! boots, after it crashes, and when it is not installed at all. Nothing in
//! this module may grow a dependency on a running image.
//!
//! One of those files is `sclang_conf.yaml`, which is how sclang itself is
//! told what to compile. Reading it is not a dependency on a running sclang
//! any more than reading a `.sc` file is — and without it the guessed
//! locations are simply wrong for anyone who has a quark checked out
//! somewhere of their own, which is the ordinary state of anybody developing
//! one.

use crate::references::ReferenceIndex;
use sclang_index::SymbolIndex;
use std::path::{Path, PathBuf};

/// What a scan turned up, for the log line after startup.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IndexStats {
    pub files: usize,
    /// Files that could not be read as UTF-8. Two in the stock class library
    /// are Latin-1 from the 2000s; they are skipped rather than guessed at.
    pub unreadable: usize,
    pub classes: usize,
    pub methods: usize,
    /// Name occurrences recorded for find-references.
    pub occurrences: usize,
}

/// Where to look for class files, and what to skip inside it.
///
/// The two travel together because sclang's own configuration writes them
/// together, and because an exclusion is usually *inside* an inclusion — the
/// reason to write one down is a single broken quark under `Extensions`, not a
/// whole root.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Roots {
    pub include: Vec<PathBuf>,
    pub exclude: Vec<PathBuf>,
}

impl Roots {
    /// Roots with nothing excluded — what an explicit setting or a workspace
    /// folder amounts to.
    pub fn of(include: Vec<PathBuf>) -> Self {
        Roots {
            include,
            exclude: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.include.len()
    }

    pub fn is_empty(&self) -> bool {
        self.include.is_empty()
    }

    /// Whether the walk should skip a directory.
    ///
    /// By prefix, so excluding a quark excludes what is under it — which is
    /// what `excludePaths` means to sclang and the only reading that is any
    /// use.
    fn excludes(&self, path: &Path) -> bool {
        self.exclude.iter().any(|e| path.starts_with(e))
    }
}

/// Where a stock SuperCollider install keeps its class library and
/// user-installed extensions, plus whatever `sclang_conf.yaml` adds.
///
/// Only paths that exist are returned, so a machine with no SuperCollider at
/// all yields an empty list and the server still starts — it just has nothing
/// but the open files to go on.
pub fn default_roots() -> Roots {
    let conf = sclang_conf_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_conf(&text))
        .unwrap_or_default();

    // `excludeDefaultPaths` turns the stock library off outright. Someone who
    // has set it is compiling a class library of their own, and indexing the
    // one they told sclang to ignore would describe a language they are not
    // writing.
    let mut candidates = if conf.exclude_defaults {
        Vec::new()
    } else {
        platform_roots()
    };
    candidates.extend(conf.include);
    candidates.retain(|p| p.is_dir());

    Roots {
        include: outermost(candidates),
        exclude: conf.exclude,
    }
}

/// Drop every root that another root already contains.
///
/// sclang names each quark it was told to compile, and a quark installed the
/// usual way sits *inside* `downloaded-quarks` — which is a root in its own
/// right. Walking both reads the same files twice, which is wasted work and a
/// file count that does not match reality.
///
/// Sorted first, so a directory sorts before anything beneath it and the
/// parent is the one kept.
fn outermost(mut candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    candidates.sort();
    candidates.dedup();

    let mut out: Vec<PathBuf> = Vec::new();
    for path in candidates {
        if out.iter().any(|kept| path.starts_with(kept)) {
            continue;
        }
        out.push(path);
    }
    out
}

/// The locations a stock install uses, before configuration is consulted.
fn platform_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();

    if cfg!(target_os = "macos") {
        candidates.push("/Applications/SuperCollider.app/Contents/Resources/SCClassLibrary".into());
        if let Some(home) = &home {
            candidates.push(home.join("Library/Application Support/SuperCollider/Extensions"));
            candidates
                .push(home.join("Library/Application Support/SuperCollider/downloaded-quarks"));
        }
    } else if cfg!(target_os = "windows") {
        candidates.push(r"C:\Program Files\SuperCollider\SCClassLibrary".into());
        if let Some(appdata) = std::env::var_os("LOCALAPPDATA") {
            let appdata = PathBuf::from(appdata);
            candidates.push(appdata.join("SuperCollider/Extensions"));
            candidates.push(appdata.join("SuperCollider/downloaded-quarks"));
        }
    } else {
        candidates.push("/usr/share/SuperCollider/SCClassLibrary".into());
        candidates.push("/usr/local/share/SuperCollider/SCClassLibrary".into());
        if let Some(home) = &home {
            candidates.push(home.join(".local/share/SuperCollider/Extensions"));
            candidates.push(home.join(".local/share/SuperCollider/downloaded-quarks"));
        }
    }

    candidates
}

/// Where sclang keeps `sclang_conf.yaml`.
///
/// The same directory sclang's own `SC_Filesystem` calls `UserConfig`, which
/// is not the platform's config directory on macOS — it is Application
/// Support, alongside `Extensions`.
pub fn sclang_conf_path() -> Option<PathBuf> {
    const NAME: &str = "sclang_conf.yaml";

    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/SuperCollider")
                .join(NAME),
        )
    } else if cfg!(target_os = "windows") {
        let appdata = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(appdata).join("SuperCollider").join(NAME))
    } else {
        // `$XDG_CONFIG_HOME` if it is set, `~/.config` otherwise — the
        // ordering SuperCollider itself uses.
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("SuperCollider").join(NAME))
    }
}

/// What `sclang_conf.yaml` says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SclangConf {
    pub include: Vec<PathBuf>,
    pub exclude: Vec<PathBuf>,
    /// `excludeDefaultPaths` — compile nothing but `includePaths`.
    pub exclude_defaults: bool,
}

/// Read the three keys that say what gets compiled.
///
/// Hand-written rather than pulled in with a YAML crate. The file has one
/// shape, because sclang writes it: top-level keys, block sequences of plain
/// scalars, and two booleans. A parser for that is thirty lines and keeps this
/// binary free of a dependency it would otherwise carry for one file — the
/// same reason `sc_lexer` is diffed against rather than linked.
///
/// Anything it does not understand is ignored rather than refused. A
/// configuration file this cannot read is not a reason to index nothing.
pub fn parse_conf(text: &str) -> SclangConf {
    let mut conf = SclangConf::default();
    let mut current: Option<&str> = None;

    for line in text.lines() {
        let line = strip_comment(line);
        if line.trim().is_empty() {
            continue;
        }

        // An item of the sequence opened by the last key. Indented, which is
        // how it is told apart from a key that happens to start with `-`.
        if line.starts_with(char::is_whitespace) {
            if let Some(value) = line.trim_start().strip_prefix('-') {
                match current {
                    Some("includePaths") => conf.include.push(as_path(value)),
                    Some("excludePaths") => conf.exclude.push(as_path(value)),
                    _ => {}
                }
                continue;
            }
            // `excludePaths:` followed by an indented `[]`, which is what
            // sclang writes for an empty list.
            continue;
        }

        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let (key, rest) = (key.trim(), rest.trim());
        current = None;

        match key {
            "includePaths" | "excludePaths" => {
                let into = if key == "includePaths" {
                    &mut conf.include
                } else {
                    &mut conf.exclude
                };
                // An inline list, which a hand-edited file may well use.
                if let Some(items) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    into.extend(
                        items
                            .split(',')
                            .filter(|i| !i.trim().is_empty())
                            .map(as_path),
                    );
                } else if rest.is_empty() {
                    // A block sequence follows.
                    current = Some(if key == "includePaths" {
                        "includePaths"
                    } else {
                        "excludePaths"
                    });
                }
            }
            "excludeDefaultPaths" => conf.exclude_defaults = rest == "true",
            _ => {}
        }
    }

    conf
}

/// Everything before an unquoted `#`, which YAML treats as a comment only
/// when something separates it from what precedes it.
fn strip_comment(line: &str) -> &str {
    match line.find(" #") {
        Some(at) => &line[..at],
        None if line.trim_start().starts_with('#') => "",
        None => line,
    }
}

/// One scalar as a path: unquoted, and with a leading `~` expanded, which is
/// what sclang's own `standardizePath` does to these.
fn as_path(value: &str) -> PathBuf {
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);

    match value.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(value),
        },
        None => PathBuf::from(value),
    }
}

/// Every `.sc` file under `root`, recursively, skipping what is excluded.
///
/// `.scd` is deliberately excluded: those are scripts meant to be evaluated,
/// not class definitions, and several in the stock library do not parse as a
/// whole file by design.
pub fn collect_sc_files(root: &Path, roots: &Roots) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if roots.excludes(root) {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if roots.excludes(&path) {
                continue;
            }
            if file_type.is_dir() {
                // Symlinked directories are not followed: quark checkouts
                // routinely link back into the class library and would make
                // this walk cyclic.
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "sc") {
                out.push(path);
            }
        }
    }

    out.sort();
    out
}

/// Index every `.sc` file under each root.
///
/// Both indexes are built from the same read, since the expensive parts are
/// reading the file and parsing it, not walking the tree twice.
pub fn build_index(roots: &Roots) -> (SymbolIndex, ReferenceIndex, IndexStats) {
    let mut index = SymbolIndex::default();
    let mut references = ReferenceIndex::default();
    let mut stats = IndexStats::default();

    for root in &roots.include {
        for path in collect_sc_files(root, roots) {
            match std::fs::read_to_string(&path) {
                Ok(source) => {
                    index.index_file(&path, &source);
                    references.index_file(&path, &source);
                    stats.files += 1;
                }
                Err(_) => stats.unreadable += 1,
            }
        }
    }

    stats.classes = index.class_count();
    stats.methods = index.method_count();
    stats.occurrences = references.occurrence_count();
    (index, references, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sclang-lsp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn collects_sc_files_recursively_and_ignores_scd() {
        let dir = temp_dir("collect");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("A.sc"), "A { }").unwrap();
        std::fs::write(dir.join("nested/B.sc"), "B { }").unwrap();
        std::fs::write(dir.join("script.scd"), "1 + 1").unwrap();

        let found = collect_sc_files(&dir, &Roots::default());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "sc"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn indexes_a_tree() {
        let dir = temp_dir("index");
        std::fs::write(dir.join("A.sc"), "A : Object { *make { |n| ^n } }").unwrap();
        std::fs::write(dir.join("B.sc"), "B : A { play { ^1 } }").unwrap();

        let (index, references, stats) = build_index(&Roots::of(vec![dir.clone()]));
        assert_eq!(stats.files, 2);
        assert!(stats.occurrences > 0);
        assert!(!references.find("A", |_| true).is_empty());
        assert_eq!(index.class("B").unwrap().superclass.as_deref(), Some("A"));
        assert_eq!(
            index
                .method("A", "make", sclang_index::MethodKind::Class)
                .unwrap()
                .signature(),
            "*make(n)"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn non_utf8_files_are_counted_not_fatal() {
        let dir = temp_dir("latin1");
        std::fs::write(dir.join("Good.sc"), "Good { }").unwrap();
        // 0xE9 is `é` in Latin-1 and invalid on its own in UTF-8.
        std::fs::write(dir.join("Bad.sc"), [b'/', b'/', 0xE9, b'\n']).unwrap();

        let (index, _references, stats) = build_index(&Roots::of(vec![dir.clone()]));
        assert_eq!(stats.files, 1);
        assert_eq!(stats.unreadable, 1);
        assert!(index.class("Good").is_some());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // =================================================================
    // sclang's own configuration
    // =================================================================

    /// A fixture as lines, not as one string: a multi-line literal would carry
    /// this file's own indentation into it, and indentation is exactly what
    /// the parser reads.
    fn conf(lines: &[&str]) -> SclangConf {
        parse_conf(&lines.join("\n"))
    }

    #[test]
    fn reads_the_include_paths_sclang_writes() {
        // Exactly what sclang 3.13 writes, down to the `[]` for an empty list.
        let parsed = conf(&[
            "includePaths:",
            "    -   /Users/me/Documents/Github/LanguageServer.quark",
            "    -   /Users/me/Library/Application Support/SuperCollider/downloaded-quarks/Singleton",
            "excludePaths:",
            "    []",
            "postInlineWarnings: false",
            "excludeDefaultPaths: false",
        ]);

        assert_eq!(
            parsed.include,
            vec![
                PathBuf::from("/Users/me/Documents/Github/LanguageServer.quark"),
                PathBuf::from(
                    "/Users/me/Library/Application Support/SuperCollider/downloaded-quarks/Singleton"
                ),
            ]
        );
        // `[]` is an empty list, not a path named `[]`.
        assert!(parsed.exclude.is_empty());
        assert!(!parsed.exclude_defaults);
    }

    #[test]
    fn reads_exclusions_and_the_default_switch() {
        let parsed = conf(&[
            "includePaths:",
            "excludePaths:",
            "    -   /opt/broken-quark",
            "excludeDefaultPaths: true",
        ]);
        assert!(parsed.include.is_empty());
        assert_eq!(parsed.exclude, vec![PathBuf::from("/opt/broken-quark")]);
        assert!(parsed.exclude_defaults);
    }

    #[test]
    fn tolerates_what_a_hand_edited_file_looks_like() {
        // A comment, a blank line, an inline list and quoted paths. None of
        // these is what sclang writes; all of them are what someone editing
        // the file themselves produces.
        let parsed = conf(&[
            "# my paths",
            "",
            "includePaths: [/a, \"/b with space\"]",
            "excludePaths:",
            "    -   '/c'   # the broken one",
        ]);
        assert_eq!(
            parsed.include,
            vec![PathBuf::from("/a"), PathBuf::from("/b with space")]
        );
        assert_eq!(parsed.exclude, vec![PathBuf::from("/c")]);
    }

    #[test]
    fn a_file_that_makes_no_sense_yields_nothing_rather_than_failing() {
        // A configuration this cannot read is not a reason to index nothing.
        for lines in [
            vec![""],
            vec!["!!!"],
            vec!["includePaths"],
            vec!["includePaths: {a: 1}"],
            vec!["    -   /orphaned/item"],
        ] {
            let parsed = conf(&lines);
            assert!(parsed.include.is_empty(), "{lines:?}");
            assert!(parsed.exclude.is_empty(), "{lines:?}");
            assert!(!parsed.exclude_defaults, "{lines:?}");
        }
    }

    #[test]
    fn a_leading_tilde_is_expanded() {
        // sclang standardizes these paths, so `~/x` names the home directory
        // rather than a directory called `~`.
        let parsed = conf(&["includePaths:", "    -   ~/quarks/Mine"]);
        let home = PathBuf::from(std::env::var_os("HOME").expect("a home directory"));
        assert_eq!(parsed.include, vec![home.join("quarks/Mine")]);
    }

    #[test]
    fn an_excluded_directory_is_not_walked() {
        let dir = temp_dir("exclude");
        std::fs::create_dir_all(dir.join("Good")).unwrap();
        std::fs::create_dir_all(dir.join("Broken/nested")).unwrap();
        std::fs::write(dir.join("Good/A.sc"), "A { }").unwrap();
        std::fs::write(dir.join("Broken/B.sc"), "B { }").unwrap();
        std::fs::write(dir.join("Broken/nested/C.sc"), "C { }").unwrap();

        // By prefix: excluding a quark excludes everything under it, which is
        // the only reading of `excludePaths` that is any use.
        let roots = Roots {
            include: vec![dir.clone()],
            exclude: vec![dir.join("Broken")],
        };
        let (index, _references, stats) = build_index(&roots);
        assert_eq!(stats.files, 1);
        assert!(index.class("A").is_some());
        assert!(index.class("B").is_none());
        assert!(index.class("C").is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
