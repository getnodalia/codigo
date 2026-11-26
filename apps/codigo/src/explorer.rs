// SPDX-License-Identifier: MIT OR Apache-2.0
//! The explorer's data: which folders are open and what is in them.
//!
//! Folders are read on a worker thread, never on the UI thread, so opening a huge folder or a
//! slow disk shows the tree at once and fills it in as the answers arrive. The visible rows are
//! rebuilt only when something changes, not on every frame.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use codigo_workspace::{read_dir, Entry};

/// Wakes the UI when a folder has been read.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

struct Loaded {
    generation: u64,
    dir: PathBuf,
    entries: Vec<Entry>,
}

pub struct Explorer {
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    /// Folders being read right now.
    loading: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    rows: Vec<Row>,
    /// Bumped by `refresh`, so an answer that was already on its way is not taken as fresh.
    generation: u64,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    repaint: Repaint,
}

/// One visible row of the tree.
pub struct Row {
    pub entry: Entry,
    pub depth: usize,
    pub open: bool,
    /// Open, but its content has not arrived yet.
    pub loading: bool,
}

impl Explorer {
    pub fn new(root: PathBuf, repaint: Repaint) -> Self {
        let (tx, rx) = channel();
        let mut e = Explorer {
            root: root.clone(),
            expanded: HashSet::new(),
            children: HashMap::new(),
            loading: HashSet::new(),
            selected: None,
            rows: Vec::new(),
            generation: 0,
            tx,
            rx,
            repaint,
        };
        e.expanded.insert(root.clone());
        e.load(&root);
        e
    }

    /// Start reading `dir` unless it is already being read.
    fn load(&mut self, dir: &Path) {
        if !self.loading.insert(dir.to_path_buf()) {
            return;
        }
        let (tx, repaint) = (self.tx.clone(), self.repaint.clone());
        let (generation, dir) = (self.generation, dir.to_path_buf());
        let spawned = std::thread::Builder::new()
            .name("codigo-explorer".into())
            .spawn(move || {
                let entries = read_dir(&dir).unwrap_or_default();
                let _ = tx.send(Loaded {
                    generation,
                    dir,
                    entries,
                });
                repaint();
            });
        if spawned.is_err() {
            // No thread to be had: show the folder as empty rather than loading for ever.
            self.loading.clear();
        }
    }

    fn apply(&mut self, done: Loaded) {
        if done.generation != self.generation {
            return;
        }
        self.loading.remove(&done.dir);
        self.children.insert(done.dir, done.entries);
    }

    /// Take the folders the worker threads have finished. True when the tree changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            self.apply(done);
            changed = true;
        }
        if changed {
            self.rebuild();
        }
        changed
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
        self.rebuild();
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
            self.expanded.insert(dir.clone());
            if !self.children.contains_key(&dir) {
                self.load(&dir);
            }
        }
        self.selected = Some(file.to_path_buf());
        self.rebuild();
    }

    /// Read every open folder again, keeping the expanded ones open. The old listing stays on
    /// screen until the new one arrives, so the tree never blinks empty.
    pub fn refresh(&mut self) {
        self.generation += 1;
        self.loading.clear();
        let open: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        for dir in open {
            if dir.is_dir() {
                self.load(&dir);
            } else {
                self.expanded.remove(&dir);
                self.children.remove(&dir);
            }
        }
        self.rebuild();
    }

    /// True while the root itself has not been read yet.
    pub fn root_loading(&self) -> bool {
        self.loading.contains(&self.root) && !self.children.contains_key(&self.root)
    }

    /// The rows to draw, below the root.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn rebuild(&mut self) {
        let mut out = Vec::new();
        self.walk(&self.root, 0, &mut out);
        self.rows = out;
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
                loading: open && self.loading.contains(&e.path),
            });
            if open && depth < 64 {
                self.walk(&e.path, depth + 1, out);
            }
        }
    }

    /// Wait for every folder being read (tests only).
    #[cfg(test)]
    fn settle(&mut self) {
        while !self.loading.is_empty() {
            let done = self
                .rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("a folder was read");
            self.apply(done);
        }
        self.rebuild();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_explorer(root: &Path) -> Explorer {
        let mut e = Explorer::new(root.to_path_buf(), Arc::new(|| {}));
        e.settle();
        e
    }

    #[test]
    fn folders_open_and_close_and_the_tree_lists_them_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "x").unwrap();
        std::fs::write(dir.path().join("b.txt"), "y").unwrap();
        let mut e = new_explorer(dir.path());
        let rows = e.rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].entry.is_dir && !rows[0].open);
        let src = dir.path().join("src");
        e.toggle(&src);
        e.settle();
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
        let mut e = new_explorer(dir.path());
        e.reveal(&dir.path().join("a/b/c.rs"));
        e.settle();
        assert_eq!(e.rows().len(), 3);
        assert_eq!(
            e.selected.as_deref(),
            Some(dir.path().join("a/b/c.rs").as_path())
        );
    }

    #[test]
    fn a_refresh_keeps_the_old_rows_until_the_new_ones_arrive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let mut e = new_explorer(dir.path());
        std::fs::write(dir.path().join("b.txt"), "y").unwrap();
        e.refresh();
        assert_eq!(e.rows().len(), 1, "the old listing stays while it reloads");
        e.settle();
        assert_eq!(e.rows().len(), 2);
    }
}
