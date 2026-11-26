// SPDX-License-Identifier: MIT OR Apache-2.0
//! Merging, and the conflicts a merge leaves in files.

use crate::{Error, Repo, Result};

/// One block of `<<<<<<<` … `>>>>>>>` in a file. Line numbers count from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: usize,
    /// The `|||||||` line of a diff3 conflict.
    pub base: Option<usize>,
    /// The `=======` line.
    pub mid: usize,
    /// The `>>>>>>>` line.
    pub end: usize,
    /// What follows `<<<<<<<`, usually `HEAD`.
    pub ours_label: String,
    /// What follows `>>>>>>>`, usually the other branch.
    pub theirs_label: String,
}

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// What the current branch has.
    Ours,
    /// What the merged branch has.
    Theirs,
    /// Both, ours first.
    Both,
}

fn marker<'a>(line: &'a str, ch: char) -> Option<&'a str> {
    let line = line.trim_end_matches(['\n', '\r']);
    let n = line.chars().take_while(|c| *c == ch).count();
    if n != 7 {
        return None;
    }
    let rest = &line[7..];
    (rest.is_empty() || rest.starts_with(' ')).then_some(rest.trim())
}

/// Every complete conflict block in `text`, in order.
pub fn conflicts(text: &str) -> Vec<Conflict> {
    let mut found = Vec::new();
    let mut open: Option<(usize, String)> = None;
    let mut base = None;
    let mut mid = None;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if let Some(label) = marker(line, '<') {
            open = Some((i, label.to_owned()));
            base = None;
            mid = None;
        } else if open.is_some() {
            if marker(line, '|').is_some() && mid.is_none() {
                base = Some(i);
            } else if marker(line, '=').is_some_and(|r| r.is_empty()) && mid.is_none() {
                mid = Some(i);
            } else if let (Some(label), Some(m)) = (marker(line, '>'), mid) {
                if let Some((start, ours_label)) = open.take() {
                    found.push(Conflict {
                        start,
                        base,
                        mid: m,
                        end: i,
                        ours_label,
                        theirs_label: label.to_owned(),
                    });
                }
                base = None;
                mid = None;
            }
        }
    }
    found
}

/// `text` with conflict number `index` replaced by the side the person chose.
pub fn resolve(text: &str, index: usize, side: Side) -> Option<String> {
    let all = conflicts(text);
    let c = all.get(index)?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let ours_end = c.base.unwrap_or(c.mid);
    let ours = lines.get(c.start + 1..ours_end)?;
    let theirs = lines.get(c.mid + 1..c.end)?;
    let mut out = String::with_capacity(text.len());
    for l in lines.get(..c.start)? {
        out.push_str(l);
    }
    let keep = |out: &mut String, part: &[&str]| {
        for l in part {
            out.push_str(l);
        }
    };
    match side {
        Side::Ours => keep(&mut out, ours),
        Side::Theirs => keep(&mut out, theirs),
        Side::Both => {
            keep(&mut out, ours);
            // A last line without a newline would run into the next side.
            if ours.last().is_some_and(|l| !l.ends_with('\n')) {
                out.push('\n');
            }
            keep(&mut out, theirs);
        }
    }
    for l in lines.get(c.end + 1..)? {
        out.push_str(l);
    }
    Some(out)
}

/// The text of one side of a conflict, to show before choosing.
pub fn side_text(text: &str, conflict: &Conflict, side: Side) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let ours_end = conflict.base.unwrap_or(conflict.mid);
    let range = match side {
        Side::Ours => conflict.start + 1..ours_end,
        Side::Theirs => conflict.mid + 1..conflict.end,
        Side::Both => conflict.start + 1..conflict.end,
    };
    lines.get(range).map(|p| p.concat()).unwrap_or_default()
}

impl Repo {
    /// Merge a branch into the current one. Conflicts are an error whose files stay marked in
    /// the working tree for [`conflicts`] to find.
    pub fn merge(&self, branch: &str) -> Result<()> {
        if branch.is_empty() || branch.starts_with('-') || branch.contains(char::is_whitespace) {
            return Err(Error::Failed(format!(
                "`{branch}` is not a valid branch name"
            )));
        }
        self.git(&["merge", "--no-edit", branch]).map(drop)
    }

    /// Whether a merge is waiting to be finished.
    pub fn merging(&self) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", "MERGE_HEAD"])
            .is_ok()
    }

    /// Give up on the merge and go back to how things were before it.
    pub fn merge_abort(&self) -> Result<()> {
        self.git(&["merge", "--abort"]).map(drop)
    }
}
