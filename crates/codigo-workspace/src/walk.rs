// SPDX-License-Identifier: MIT OR Apache-2.0
//! Listing every file of a project, for quick open and search.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::WalkBuilder;

/// A walker for a project: hidden files are included (`.github`, `.env.example`), ignore files
/// are honoured with or without a git repository, `.git` is skipped, and links are not followed
/// so a cycle cannot trap it.
pub(crate) fn walker(root: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
    b.hidden(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| e.file_name() != ".git");
    b
}

/// Files under `root`, as paths relative to it, in walk order. Stops at `limit` files or when
/// `cancel` is set, whichever comes first.
pub fn list_files(root: &Path, limit: usize, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walker(root).build() {
        if cancel.load(Ordering::Relaxed) || out.len() >= limit {
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::create_dir_all(p.join("src")).expect("src");
        fs::create_dir_all(p.join("target/debug")).expect("target");
        fs::create_dir_all(p.join(".git/objects")).expect(".git");
        fs::create_dir_all(p.join(".github/workflows")).expect(".github");
        fs::write(p.join(".gitignore"), "target/\n").expect("ignore");
        fs::write(p.join("src/main.rs"), "fn main() {}").expect("main");
        fs::write(p.join("target/debug/app"), "x").expect("app");
        fs::write(p.join(".git/HEAD"), "ref").expect("head");
        fs::write(p.join(".github/workflows/ci.yml"), "x").expect("ci");
        d
    }

    #[test]
    fn lists_project_files_but_not_ignored_ones_or_git() {
        let d = project();
        let files = list_files(d.path(), 100, &AtomicBool::new(false));
        let names: Vec<_> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [".github/workflows/ci.yml", ".gitignore", "src/main.rs"]
        );
    }

    #[test]
    fn respects_the_limit_and_the_cancel_flag() {
        let d = project();
        assert_eq!(list_files(d.path(), 1, &AtomicBool::new(false)).len(), 1);
        assert!(list_files(d.path(), 100, &AtomicBool::new(true)).is_empty());
    }
}
