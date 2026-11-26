// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading `git status --porcelain=v2 --branch -z`.

/// What happened to a file, for the icon and the word beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

/// One changed file. `index` is its state in the next commit, `worktree` in the folder; a dot
/// in git's own output (unchanged) is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Relative to the repository root, with `/` separators.
    pub path: String,
    pub index: Option<char>,
    pub worktree: Option<char>,
    /// Where a renamed file came from.
    pub from: Option<String>,
    pub conflicted: bool,
}

impl Change {
    pub fn staged(&self) -> bool {
        self.index.is_some() && !self.conflicted && self.index != Some('?')
    }

    pub fn unstaged(&self) -> bool {
        self.conflicted || self.worktree.is_some()
    }

    pub fn kind(&self) -> Kind {
        if self.conflicted {
            return Kind::Conflicted;
        }
        match (self.index, self.worktree) {
            (Some('?'), _) | (_, Some('?')) => Kind::Untracked,
            (Some('R' | 'C'), _) | (_, Some('R' | 'C')) => Kind::Renamed,
            (Some('A'), _) => Kind::Added,
            (Some('D'), _) | (_, Some('D')) => Kind::Deleted,
            _ => Kind::Modified,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
}

impl RepoStatus {
    pub fn staged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.unstaged())
    }
}

fn state(c: char) -> Option<char> {
    (c != '.').then_some(c)
}

pub fn parse(text: &str) -> RepoStatus {
    let mut status = RepoStatus::default();
    let mut records = text.split('\0');
    while let Some(rec) = records.next() {
        if let Some(header) = rec.strip_prefix("# ") {
            header_line(&mut status, header);
        } else if let Some(rest) = rec.strip_prefix("1 ") {
            // 1 XY sub mH mI mW hH hI path
            let mut f = rest.splitn(8, ' ');
            let xy = f.next().unwrap_or("..");
            if let Some(path) = f.nth(6) {
                status.changes.push(change(xy, path, None, false));
            }
        } else if let Some(rest) = rec.strip_prefix("2 ") {
            // 2 XY sub mH mI mW hH hI Xscore path, then the original path as its own record
            let mut f = rest.splitn(9, ' ');
            let xy = f.next().unwrap_or("..");
            let path = f.nth(7);
            let from = records.next().map(str::to_owned);
            if let Some(path) = path {
                status.changes.push(change(xy, path, from, false));
            }
        } else if let Some(rest) = rec.strip_prefix("u ") {
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            let mut f = rest.splitn(10, ' ');
            let xy = f.next().unwrap_or("UU");
            if let Some(path) = f.nth(8) {
                status.changes.push(change(xy, path, None, true));
            }
        } else if let Some(path) = rec.strip_prefix("? ") {
            status.changes.push(Change {
                path: path.to_owned(),
                index: None,
                worktree: Some('?'),
                from: None,
                conflicted: false,
            });
        }
    }
    status.changes.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

fn change(xy: &str, path: &str, from: Option<String>, conflicted: bool) -> Change {
    let mut c = xy.chars();
    Change {
        path: path.to_owned(),
        index: c.next().and_then(state),
        worktree: c.next().and_then(state),
        from,
        conflicted,
    }
}

fn header_line(status: &mut RepoStatus, header: &str) {
    if let Some(head) = header.strip_prefix("branch.head ") {
        status.branch = (head != "(detached)").then(|| head.to_owned());
    } else if let Some(up) = header.strip_prefix("branch.upstream ") {
        status.upstream = Some(up.to_owned());
    } else if let Some(ab) = header.strip_prefix("branch.ab ") {
        let mut p = ab.split(' ');
        let num = |s: Option<&str>, sign: char| {
            s.and_then(|s| s.strip_prefix(sign))
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        status.ahead = num(p.next(), '+');
        status.behind = num(p.next(), '-');
    }
}
