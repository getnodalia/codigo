// SPDX-License-Identifier: MIT OR Apache-2.0
//! Staging, unstaging and discarding one hunk of a file instead of all of it.

use crate::{run, Auth, Error, Repo, Result};

/// One `@@` block of a unified diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ …` line.
    pub header: String,
    /// The lines after the header, each with its leading ` `, `+` or `-`.
    pub lines: Vec<String>,
}

impl Hunk {
    pub fn added(&self) -> usize {
        self.lines.iter().filter(|l| l.starts_with('+')).count()
    }

    pub fn removed(&self) -> usize {
        self.lines.iter().filter(|l| l.starts_with('-')).count()
    }
}

/// A file's diff taken apart: the lines before the first hunk, then the hunks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// A patch with the file header and only hunk `index`, for `git apply`.
    pub fn patch(&self, index: usize) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let mut text = String::new();
        for l in &self.header {
            text.push_str(l);
            text.push('\n');
        }
        text.push_str(&hunk.header);
        text.push('\n');
        for l in &hunk.lines {
            text.push_str(l);
            text.push('\n');
        }
        Some(text)
    }
}

/// Take apart the text `git diff` printed for one file.
pub fn parse(text: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    for line in text.lines() {
        if line.starts_with("@@ ") {
            diff.hunks.push(Hunk {
                header: line.to_owned(),
                lines: Vec::new(),
            });
        } else if let Some(h) = diff.hunks.last_mut() {
            // "\ No newline at end of file" belongs to the line before it.
            if matches!(line.chars().next(), Some(' ' | '+' | '-' | '\\')) || line.is_empty() {
                h.lines.push(line.to_owned());
            }
        } else {
            diff.header.push(line.to_owned());
        }
    }
    diff
}

impl Repo {
    fn file_diff(&self, path: &str, staged: bool) -> Result<FileDiff> {
        check_path(path)?;
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "--unified=3"];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", path]);
        Ok(parse(&self.git(&args)?))
    }

    /// The hunks of one file, in the working tree or (`staged`) in the next commit.
    pub fn hunks(&self, path: &str, staged: bool) -> Result<FileDiff> {
        self.file_diff(path, staged)
    }

    fn apply_hunk(&self, path: &str, staged_side: bool, index: usize, args: &[&str]) -> Result<()> {
        let diff = self.file_diff(path, staged_side)?;
        let patch = diff
            .patch(index)
            .ok_or_else(|| Error::Failed("that change is no longer there; refresh".into()))?;
        let mut full = vec!["apply", "--recount", "--whitespace=nowarn"];
        full.extend_from_slice(args);
        full.push("-");
        run::git_input(&self.root, &full, Some(patch.as_bytes()), &Auth::None).map(drop)
    }

    /// Put one hunk of a file's changes in the next commit.
    pub fn stage_hunk(&self, path: &str, index: usize) -> Result<()> {
        self.apply_hunk(path, false, index, &["--cached"])
    }

    /// Take one hunk back out of the next commit, keeping it in the working tree.
    pub fn unstage_hunk(&self, path: &str, index: usize) -> Result<()> {
        self.apply_hunk(path, true, index, &["--cached", "--reverse"])
    }

    /// Throw away one hunk of the working tree.
    pub fn discard_hunk(&self, path: &str, index: usize) -> Result<()> {
        self.apply_hunk(path, false, index, &["--reverse"])
    }
}

fn check_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('-') || path.contains('\0') {
        Err(Error::Failed(format!(
            "`{path}` is not a file in the project"
        )))
    } else {
        Ok(())
    }
}
