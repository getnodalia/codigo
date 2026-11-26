// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's version control, as a library with no interface.
//!
//! Git is only ever run as a program, with arguments and never through a shell. It never asks a
//! question on the terminal (`GIT_TERMINAL_PROMPT=0`), so a missing credential is an error to
//! show rather than a hang, and `core.fsmonitor` is off so opening a folder cannot run a program
//! its config names. A token for a remote reaches git through the environment of that one
//! process, never through its arguments or a file.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod base64;
mod blame;
mod hunks;
mod merge;
mod run;
mod stash;
mod status;

use std::path::{Path, PathBuf};

pub use blame::BlameLine;
pub use hunks::{parse as parse_diff, FileDiff, Hunk};
pub use merge::{conflicts, resolve, side_text, Conflict, Side};
pub use run::Auth;
pub use stash::Stash;
pub use status::{Change, Kind, RepoStatus};

/// What can go wrong running git.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("git is not installed, or could not be started: {0}")]
    NotInstalled(String),
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A branch of the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    /// A remote-tracking branch such as `origin/main`.
    pub remote: bool,
}

/// One commit of the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Seconds since the epoch.
    pub time: i64,
    pub subject: String,
}

/// A working tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

/// Longest diff returned, so a generated file cannot fill the window.
const MAX_DIFF: usize = 512 * 1024;

impl Repo {
    /// The repository `dir` is in, if any.
    pub fn discover(dir: &Path) -> Option<Repo> {
        let out = run::git(dir, &["rev-parse", "--show-toplevel"], &Auth::None).ok()?;
        let top = out.trim();
        (!top.is_empty()).then(|| Repo {
            root: PathBuf::from(top),
        })
    }

    /// Make a new repository in `dir`.
    pub fn init(dir: &Path) -> Result<Repo> {
        run::git(dir, &["init", "--quiet"], &Auth::None)?;
        Repo::discover(dir).ok_or_else(|| Error::Failed("the new repository was not found".into()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        run::git(&self.root, args, &Auth::None)
    }

    /// Branch, upstream and every changed file, staged or not.
    pub fn status(&self) -> Result<RepoStatus> {
        let out = self.git(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Ok(status::parse(&out))
    }

    /// Put these files (paths relative to the root) in the next commit.
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        self.with_paths(&["add", "--"], paths)
    }

    pub fn stage_all(&self) -> Result<()> {
        self.git(&["add", "--all"]).map(drop)
    }

    /// Take these files out of the next commit, keeping their changes.
    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if self.has_commits() {
            self.with_paths(&["restore", "--staged", "--"], paths)
        } else {
            self.with_paths(&["rm", "--cached", "-r", "--quiet", "--"], paths)
        }
    }

    /// Throw away the changes of a file in the working tree. An untracked file is deleted.
    pub fn discard(&self, change: &Change) -> Result<()> {
        let path = vec![change.path.clone()];
        if change.kind() == Kind::Untracked {
            self.with_paths(&["clean", "-f", "--"], &path)
        } else {
            self.with_paths(
                &["restore", "--source=HEAD", "--staged", "--worktree", "--"],
                &path,
            )
        }
    }

    fn with_paths(&self, head: &[&str], paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = head.to_vec();
        args.extend(paths.iter().map(String::as_str));
        self.git(&args).map(drop)
    }

    fn has_commits(&self) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok()
    }

    /// Commit what is staged. Returns the new commit's short hash.
    pub fn commit(&self, message: &str) -> Result<String> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::Failed("write a commit message first".into()));
        }
        self.git(&["commit", "--quiet", "-m", message])?;
        self.git(&["rev-parse", "--short", "HEAD"])
            .map(|s| s.trim().to_owned())
    }

    /// Local branches first, then remote-tracking ones.
    pub fn branches(&self) -> Result<Vec<Branch>> {
        let out = self.git(&[
            "branch",
            "--all",
            "--format=%(HEAD)|%(refname)|%(refname:short)",
        ])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '|');
                let (head, full, short) = (p.next()?, p.next()?, p.next()?);
                if short.is_empty() || short.ends_with("/HEAD") || short.starts_with('(') {
                    return None;
                }
                Some(Branch {
                    name: short.to_owned(),
                    current: head == "*",
                    remote: full.starts_with("refs/remotes/"),
                })
            })
            .collect())
    }

    /// Switch to a branch. A remote-tracking name (`origin/fix`) makes a local branch of it.
    pub fn checkout(&self, name: &str) -> Result<()> {
        check_ref(name)?;
        self.git(&["switch", "--quiet", name]).map(drop)
    }

    /// Make a branch from the current commit and switch to it.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        check_ref(name)?;
        self.git(&["switch", "--quiet", "--create", name]).map(drop)
    }

    /// The most recent commits.
    pub fn log(&self, count: usize) -> Result<Vec<Commit>> {
        if !self.has_commits() {
            return Ok(Vec::new());
        }
        let n = format!("-{}", count.clamp(1, 500));
        let out = self.git(&["log", &n, "--format=%H%x1f%h%x1f%an%x1f%ct%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\u{1f}');
                Some(Commit {
                    hash: p.next()?.to_owned(),
                    short: p.next()?.to_owned(),
                    author: p.next()?.to_owned(),
                    time: p.next()?.parse().ok()?,
                    subject: p.next()?.to_owned(),
                })
            })
            .collect())
    }

    /// The diff of one file, staged or not, capped in size.
    pub fn diff(&self, change: &Change, staged: bool) -> Result<String> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", change.path.as_str()]);
        let mut text = if change.kind() == Kind::Untracked && !staged {
            // An untracked file has no diff; show it all as added.
            self.git(&[
                "diff",
                "--no-color",
                "--no-index",
                "--",
                null_device(),
                change.path.as_str(),
            ])
            .or_else(|e| match e {
                Error::Failed(text) => Ok(text),
                other => Err(other),
            })?
        } else {
            self.git(&args)?
        };
        if text.len() > MAX_DIFF {
            let mut cut = MAX_DIFF;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n… (the diff is too long to show)\n");
        }
        Ok(text)
    }

    /// The URL of a remote, with any credentials removed.
    pub fn remote_url(&self, remote: &str) -> Option<String> {
        let url = self.git(&["remote", "get-url", remote]).ok()?;
        Some(strip_credentials(url.trim()))
    }

    /// Download from the remote without changing the working tree.
    pub fn fetch(&self, auth: &Auth) -> Result<()> {
        run::git(&self.root, &["fetch", "--quiet", "--prune"], auth).map(drop)
    }

    /// Fetch one ref from `origin` into a local branch, e.g. `pull/12/head:codigo-pr-12`.
    pub fn fetch_ref(&self, auth: &Auth, spec: &str) -> Result<()> {
        if spec.starts_with('-') || spec.contains(char::is_whitespace) {
            return Err(Error::Failed(format!("`{spec}` is not a valid ref")));
        }
        run::git(
            &self.root,
            &["fetch", "--quiet", "--force", "origin", spec],
            auth,
        )
        .map(drop)
    }

    /// Fetch and fast-forward; never makes a merge commit by itself.
    pub fn pull(&self, auth: &Auth) -> Result<()> {
        run::git(&self.root, &["pull", "--quiet", "--ff-only"], auth).map(drop)
    }

    /// Send the branch to its remote, setting its upstream the first time.
    pub fn push(&self, auth: &Auth) -> Result<()> {
        let status = self.status()?;
        if status.upstream.is_some() {
            run::git(&self.root, &["push", "--quiet"], auth).map(drop)
        } else {
            run::git(
                &self.root,
                &["push", "--quiet", "--set-upstream", "origin", "HEAD"],
                auth,
            )
            .map(drop)
        }
    }
}

/// Clone `url` into a new folder `name` inside `parent`. Returns the new folder.
pub fn clone(url: &str, parent: &Path, name: &str, auth: &Auth) -> Result<PathBuf> {
    check_clone_url(url)?;
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(Error::Failed("that is not a folder name".into()));
    }
    let dest = parent.join(name);
    if dest.exists() {
        return Err(Error::Failed(format!("{} already exists", dest.display())));
    }
    run::git(parent, &["clone", "--quiet", "--", url, name], auth)?;
    Ok(dest)
}

/// The folder name git would use for a URL: `https://github.com/a/b.git` is `b`.
pub fn folder_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

/// Only addresses that name a remote are cloned; an argument that starts with `-` could be a
/// git option and a `file` or `ext::` transport can run programs.
fn check_clone_url(url: &str) -> Result<()> {
    let ok = url.starts_with("https://")
        || url.starts_with("ssh://")
        || url.starts_with("git@")
        || url.starts_with("http://");
    if ok && !url.contains(char::is_whitespace) {
        Ok(())
    } else {
        Err(Error::Failed(
            "enter an https:// or ssh address, or an owner/name from GitHub".into(),
        ))
    }
}

fn check_ref(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') || name.contains(char::is_whitespace) {
        Err(Error::Failed(format!(
            "`{name}` is not a valid branch name"
        )))
    } else {
        Ok(())
    }
}

fn strip_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host)) if !rest.split('/').next().unwrap_or("").is_empty() => {
                if rest.split('/').next().is_some_and(|h| h.contains('@')) {
                    format!("{scheme}://{host}")
                } else {
                    url.to_owned()
                }
            }
            _ => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

#[cfg(test)]
mod tests;
