// SPDX-License-Identifier: MIT OR Apache-2.0
//! What git says about a project: the branch, which files changed, and which lines.
//!
//! Git is only ever run as a command, with arguments and never through a shell, and with
//! `core.fsmonitor` off so opening a folder cannot run a program its config names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Clone, Debug, Default)]
pub struct GitInfo {
    /// The top of the working tree.
    pub root: PathBuf,
    /// The branch, or `None` for a detached HEAD.
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    files: HashMap<PathBuf, Status>,
    dirty_dirs: HashSet<PathBuf>,
}

impl GitInfo {
    /// The status of a file, or, for a folder, `Modified` when anything inside it changed.
    /// `path` is absolute.
    pub fn status_of(&self, path: &Path) -> Option<Status> {
        self.files
            .get(path)
            .copied()
            .or_else(|| self.dirty_dirs.contains(path).then_some(Status::Modified))
    }

    pub fn changed_files(&self) -> impl Iterator<Item = (&PathBuf, &Status)> {
        self.files.iter()
    }

    pub fn is_clean(&self) -> bool {
        self.files.is_empty()
    }
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=false"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    c
}

fn run(mut command: Command) -> Option<Vec<u8>> {
    let out = command.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// The git state of the repository containing `dir`, or `None` when there is none or git is not
/// installed.
pub fn status(dir: &Path) -> Option<GitInfo> {
    let top = run({
        let mut c = git(dir);
        c.args(["rev-parse", "--show-toplevel"]);
        c
    })?;
    let root = PathBuf::from(String::from_utf8_lossy(&top).trim_end());
    let raw = run({
        let mut c = git(&root);
        c.args([
            "status",
            "--porcelain=v1",
            "-z",
            "-b",
            "--untracked-files=all",
        ]);
        c
    })?;
    Some(parse_status(&root, &raw))
}

pub(crate) fn parse_status(root: &Path, raw: &[u8]) -> GitInfo {
    let text = String::from_utf8_lossy(raw);
    let mut info = GitInfo {
        root: root.to_path_buf(),
        ..GitInfo::default()
    };
    let mut parts = text.split('\0');
    while let Some(entry) = parts.next() {
        if let Some(header) = entry.strip_prefix("## ") {
            parse_branch(header, &mut info);
            continue;
        }
        let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        let mut chars = code.chars();
        let (x, y) = (chars.next().unwrap_or(' '), chars.next().unwrap_or(' '));
        let status = match (x, y) {
            ('?', '?') => Status::Untracked,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Status::Conflicted,
            ('R', _) | ('C', _) => Status::Renamed,
            ('A', _) => Status::Added,
            ('D', _) | (_, 'D') => Status::Deleted,
            _ => Status::Modified,
        };
        if matches!(x, 'R' | 'C') {
            // A rename lists the old name as the next entry.
            parts.next();
        }
        let full = root.join(path);
        let mut parent = full.parent();
        while let Some(dir) = parent {
            if dir == root || !info.dirty_dirs.insert(dir.to_path_buf()) {
                break;
            }
            parent = dir.parent();
        }
        info.files.insert(full, status);
    }
    info
}

fn parse_branch(header: &str, info: &mut GitInfo) {
    if header.starts_with("HEAD (no branch)") {
        return;
    }
    let (names, tracking) = match header.split_once(" [") {
        Some((n, t)) => (n, t.trim_end_matches(']')),
        None => (header, ""),
    };
    let local = names.split("...").next().unwrap_or(names);
    let local = local.strip_prefix("No commits yet on ").unwrap_or(local);
    info.branch = Some(local.to_string());
    for item in tracking.split(", ") {
        if let Some(n) = item.strip_prefix("ahead ") {
            info.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = item.strip_prefix("behind ") {
            info.behind = n.parse().unwrap_or(0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChangeKind {
    Added,
    Modified,
    /// Lines were deleted just before `line`.
    Removed,
}

/// A run of changed lines, for the editor's gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineChange {
    /// 0-based first line in the working copy.
    pub line: usize,
    pub count: usize,
    pub kind: LineChangeKind,
}

/// How the working copy of `file` differs from the index, line by line. Empty when the file is
/// untracked, unchanged or not in a repository.
pub fn line_changes(file: &Path) -> Vec<LineChange> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return Vec::new();
    };
    let mut c = git(dir);
    c.args(["diff", "-U0", "--no-color", "--no-ext-diff", "--"])
        .arg(name);
    run(c)
        .map(|out| parse_hunks(&String::from_utf8_lossy(&out)))
        .unwrap_or_default()
}

pub(crate) fn parse_hunks(diff: &str) -> Vec<LineChange> {
    let mut out = Vec::new();
    for line in diff.lines() {
        let Some(rest) = line.strip_prefix("@@ -") else {
            continue;
        };
        let Some((old, rest)) = rest.split_once(" +") else {
            continue;
        };
        let new = rest.split(" @@").next().unwrap_or(rest);
        let (_, old_count) = range(old);
        let (start, count) = range(new);
        let change = if count == 0 {
            // `start` is the line before the deletion, 1-based.
            LineChange {
                line: start,
                count: 0,
                kind: LineChangeKind::Removed,
            }
        } else {
            LineChange {
                line: start.saturating_sub(1),
                count,
                kind: if old_count == 0 {
                    LineChangeKind::Added
                } else {
                    LineChangeKind::Modified
                },
            }
        };
        out.push(change);
    }
    out
}

/// `12,3` or `12`: first line and count, the count defaulting to 1.
fn range(text: &str) -> (usize, usize) {
    match text.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (text.parse().unwrap_or(0), 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_tracking_and_every_kind_of_status() {
        let root = Path::new("/p");
        let raw = b"## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0A  src/new.rs\0?? notes.txt\0 D gone.rs\0R  new name.rs\0old name.rs\0UU merge.rs\0";
        let g = parse_status(root, raw);
        assert_eq!(g.branch.as_deref(), Some("main"));
        assert_eq!((g.ahead, g.behind), (2, 1));
        assert_eq!(
            g.status_of(Path::new("/p/src/a.rs")),
            Some(Status::Modified)
        );
        assert_eq!(g.status_of(Path::new("/p/src/new.rs")), Some(Status::Added));
        assert_eq!(
            g.status_of(Path::new("/p/notes.txt")),
            Some(Status::Untracked)
        );
        assert_eq!(g.status_of(Path::new("/p/gone.rs")), Some(Status::Deleted));
        assert_eq!(
            g.status_of(Path::new("/p/new name.rs")),
            Some(Status::Renamed)
        );
        assert_eq!(g.status_of(Path::new("/p/old name.rs")), None);
        assert_eq!(
            g.status_of(Path::new("/p/merge.rs")),
            Some(Status::Conflicted)
        );
        // A folder is dirty when something inside it is, but the root itself is not listed.
        assert_eq!(g.status_of(Path::new("/p/src")), Some(Status::Modified));
        assert_eq!(g.status_of(Path::new("/p")), None);
        assert_eq!(g.status_of(Path::new("/p/other")), None);
        assert!(!g.is_clean());
    }

    #[test]
    fn branch_headers_of_odd_repositories() {
        let mut g = GitInfo::default();
        parse_branch("feature/x", &mut g);
        assert_eq!(g.branch.as_deref(), Some("feature/x"));
        let mut g = GitInfo::default();
        parse_branch("No commits yet on trunk", &mut g);
        assert_eq!(g.branch.as_deref(), Some("trunk"));
        let mut g = GitInfo::default();
        parse_branch("HEAD (no branch)", &mut g);
        assert_eq!(g.branch, None);
        let mut g = GitInfo::default();
        parse_branch("dev...origin/dev [gone]", &mut g);
        assert_eq!(
            (g.branch.as_deref(), g.ahead, g.behind),
            (Some("dev"), 0, 0)
        );
    }

    #[test]
    fn garbage_status_output_is_ignored() {
        let g = parse_status(Path::new("/p"), b"\0x\0??\0\xff\xfe\0 M a\0");
        assert!(g.is_clean() || g.changed_files().count() <= 1);
    }

    #[test]
    fn hunks_become_gutter_marks() {
        let diff = "diff --git a/f b/f\n@@ -3,0 +4,2 @@ fn\n+a\n+b\n@@ -10,2 +12,2 @@\n-x\n-y\n+X\n+Y\n@@ -20,3 +21,0 @@\n-gone\n@@ -30 +31 @@\n-a\n+b\n";
        assert_eq!(
            parse_hunks(diff),
            [
                LineChange {
                    line: 3,
                    count: 2,
                    kind: LineChangeKind::Added
                },
                LineChange {
                    line: 11,
                    count: 2,
                    kind: LineChangeKind::Modified
                },
                LineChange {
                    line: 21,
                    count: 0,
                    kind: LineChangeKind::Removed
                },
                LineChange {
                    line: 30,
                    count: 1,
                    kind: LineChangeKind::Modified
                },
            ]
        );
        assert!(parse_hunks("@@ nonsense\n@@ -x +y @@\n").len() <= 1);
    }

    fn have_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn talks_to_a_real_repository() {
        if !have_git() {
            return;
        }
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        let sh = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(p)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "git {args:?}");
        };
        sh(&["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "1\n2\n3\n").expect("a");
        sh(&["add", "."]);
        sh(&["commit", "-qm", "first"]);
        std::fs::write(p.join("a.txt"), "1\nTWO\n3\n4\n").expect("edit");
        std::fs::write(p.join("new.txt"), "x").expect("new");

        let g = status(p).expect("repository");
        assert_eq!(g.branch.as_deref(), Some("main"));
        let root = std::fs::canonicalize(p).expect("canonical");
        assert_eq!(g.status_of(&root.join("a.txt")), Some(Status::Modified));
        assert_eq!(g.status_of(&root.join("new.txt")), Some(Status::Untracked));

        let changes = line_changes(&p.join("a.txt"));
        assert_eq!(
            changes,
            [
                LineChange {
                    line: 1,
                    count: 1,
                    kind: LineChangeKind::Modified
                },
                LineChange {
                    line: 3,
                    count: 1,
                    kind: LineChangeKind::Added
                },
            ]
        );
        assert!(line_changes(&p.join("new.txt")).is_empty());
    }

    #[test]
    fn a_folder_that_is_not_a_repository_gives_none() {
        let d = tempfile::tempdir().expect("tempdir");
        // The temp dir may itself sit inside a repository on odd machines; only assert the
        // call is safe, and that a missing directory is None.
        let _ = status(d.path());
        assert!(status(Path::new("/definitely/not/here")).is_none());
        assert!(line_changes(Path::new("/definitely/not/here/f")).is_empty());
    }
}
