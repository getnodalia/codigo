// SPDX-License-Identifier: MIT OR Apache-2.0
//! The explorer's data: which folders are open and what is in them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use codigo_workspace::{read_dir, Entry};

pub struct Explorer {
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    pub selected: Option<PathBuf>,
}

/// One visible row of the tree.
pub struct Row {
    pub entry: Entry,
    pub depth: usize,
    pub open: bool,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let mut e = Explorer {
            root: root.clone(),
            expanded: HashSet::new(),
            children: HashMap::new(),
            selected: None,
        };
        e.expanded.insert(root.clone());
        e.load(&root);
        e
    }

    fn load(&mut self, dir: &Path) {
        let entries = read_dir(dir).unwrap_or_default();
        self.children.insert(dir.to_path_buf(), entries);
    }

    pub fn toggle(&mut self, dir: &Path) {
        if self.expanded.contains(dir) {
            self.expanded.remove(dir);
        } else {
            self.expanded.insert(dir.to_path_buf());
            if !self.children.contains_key(dir) {
                self.load(dir);
            }
        }
    }

    pub fn reveal(&mut self, file: &Path) {
        let mut chain = Vec::new();
        let mut cur = file.parent();
        while let Some(dir) = cur {
            if !dir.starts_with(&self.root) {
                break;
            }
            chain.push(dir.to_path_buf());
            cur = dir.parent();
        }
        for dir in chain.into_iter().rev() {
            if !self.expanded.contains(&dir) {
                self.expanded.insert(dir.clone());
            }
            if !self.children.contains_key(&dir) {
                self.load(&dir);
            }
        }
        self.selected = Some(file.to_path_buf());
    }

    /// Read every open folder again, keeping the expanded ones open.
    pub fn refresh(&mut self) {
        let open: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        self.children.clear();
        for dir in open {
            if dir.is_dir() {
                self.load(&dir);
            } else {
                self.expanded.remove(&dir);
            }
        }
    }

    /// The rows to draw, below the root.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        self.walk(&self.root, 0, &mut out);
        out
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for e in entries {
            let open = e.is_dir && self.expanded.contains(&e.path);
            out.push(Row {
                entry: e.clone(),
                depth,
                open,
            });
            if open && depth < 64 {
                self.walk(&e.path, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_open_and_close_and_the_tree_lists_them_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "x").unwrap();
        std::fs::write(dir.path().join("b.txt"), "y").unwrap();
        let mut e = Explorer::new(dir.path().to_path_buf());
        let rows = e.rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].entry.is_dir && !rows[0].open);
        let src = dir.path().join("src");
        e.toggle(&src);
        let rows = e.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].depth, 1);
        e.toggle(&src);
        assert_eq!(e.rows().len(), 2);
    }

    #[test]
    fn revealing_a_file_opens_its_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/c.rs"), "x").unwrap();
        let mut e = Explorer::new(dir.path().to_path_buf());
        e.reveal(&dir.path().join("a/b/c.rs"));
        assert_eq!(e.rows().len(), 3);
        assert_eq!(
            e.selected.as_deref(),
            Some(dir.path().join("a/b/c.rs").as_path())
        );
    }
}
