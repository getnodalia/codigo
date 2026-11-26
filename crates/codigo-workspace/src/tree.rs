// SPDX-License-Identifier: MIT OR Apache-2.0
//! One folder of the file tree.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// A file or folder in the explorer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Matched by a `.gitignore` or similar: shown, but dimmed.
    pub ignored: bool,
}

/// The children of `dir`: folders first, then files, each in case-insensitive name order.
/// `.git` is never listed. Entries that ignore rules hide from searches are marked `ignored`
/// rather than dropped, so the explorer can still show them.
pub fn read_dir(dir: &Path) -> io::Result<Vec<Entry>> {
    let visible: HashSet<PathBuf> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.depth() == 1)
        .map(|e| e.into_path())
        .collect();

    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = item.path();
        let Ok(kind) = item.file_type() else { continue };
        let is_symlink = kind.is_symlink();
        // A link to a folder opens like a folder.
        let is_dir = if is_symlink {
            std::fs::metadata(&path).is_ok_and(|m| m.is_dir())
        } else {
            kind.is_dir()
        };
        let ignored = !visible.contains(&path);
        out.push(Entry {
            path,
            name,
            is_dir,
            is_symlink,
            ignored,
        });
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn folders_first_then_names_without_case_and_git_is_hidden() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::create_dir(p.join(".git")).expect("git");
        fs::create_dir(p.join("zeta")).expect("zeta");
        fs::create_dir(p.join("Alpha")).expect("alpha");
        fs::write(p.join("b.txt"), "").expect("b");
        fs::write(p.join("A.txt"), "").expect("a");
        let names: Vec<_> = read_dir(p)
            .expect("read")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt"]);
    }

    #[test]
    fn gitignored_entries_are_marked_not_dropped() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::write(p.join(".gitignore"), "target/\n*.log\n").expect("ignore");
        fs::create_dir(p.join("target")).expect("target");
        fs::write(p.join("run.log"), "").expect("log");
        fs::write(p.join("main.rs"), "").expect("main");
        let entries = read_dir(p).expect("read");
        let get = |n: &str| entries.iter().find(|e| e.name == n).expect("entry");
        assert!(get("target").ignored && get("target").is_dir);
        assert!(get("run.log").ignored);
        assert!(!get("main.rs").ignored);
        assert!(!get(".gitignore").ignored);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        assert!(read_dir(Path::new("/definitely/not/here")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_folder_is_a_folder() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::create_dir(p.join("real")).expect("real");
        std::os::unix::fs::symlink(p.join("real"), p.join("link")).expect("link");
        let entries = read_dir(p).expect("read");
        let link = entries.iter().find(|e| e.name == "link").expect("link");
        assert!(link.is_dir && link.is_symlink);
    }
}
