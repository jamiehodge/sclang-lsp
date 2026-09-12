//! Finding SuperCollider source on disk and indexing it.
//!
//! This is tier 1 from ARCHITECTURE.md: everything here works by reading
//! files, so it is available before sclang boots, after it crashes, and when
//! it is not installed. Nothing in this module may grow a dependency on a
//! running image.

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

/// Where a stock SuperCollider install keeps its class library and
/// user-installed extensions.
///
/// Only paths that exist are returned, so a machine with no SuperCollider at
/// all yields an empty list and the server still starts — it just has nothing
/// but the open files to go on.
pub fn default_roots() -> Vec<PathBuf> {
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

    candidates.retain(|p| p.is_dir());
    candidates
}

/// Every `.sc` file under `root`, recursively.
///
/// `.scd` is deliberately excluded: those are scripts meant to be evaluated,
/// not class definitions, and several in the stock library do not parse as a
/// whole file by design.
pub fn collect_sc_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
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
pub fn build_index(roots: &[PathBuf]) -> (SymbolIndex, ReferenceIndex, IndexStats) {
    let mut index = SymbolIndex::default();
    let mut references = ReferenceIndex::default();
    let mut stats = IndexStats::default();

    for root in roots {
        for path in collect_sc_files(root) {
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

        let found = collect_sc_files(&dir);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "sc"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn indexes_a_tree() {
        let dir = temp_dir("index");
        std::fs::write(dir.join("A.sc"), "A : Object { *make { |n| ^n } }").unwrap();
        std::fs::write(dir.join("B.sc"), "B : A { play { ^1 } }").unwrap();

        let (index, references, stats) = build_index(std::slice::from_ref(&dir));
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

        let (index, _references, stats) = build_index(std::slice::from_ref(&dir));
        assert_eq!(stats.files, 1);
        assert_eq!(stats.unreadable, 1);
        assert!(index.class("Good").is_some());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
