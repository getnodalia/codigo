// SPDX-License-Identifier: MIT OR Apache-2.0
//! Stashes: setting changes aside and bringing them back.

use crate::{Error, Repo, Result};

/// One entry of `git stash list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    /// Its place in the list: 0 is the newest.
    pub index: usize,
    /// What the person wrote, or git's own "WIP on branch: …".
    pub message: String,
    /// Seconds since the epoch.
    pub time: i64,
}

impl Repo {
    /// Every stash, newest first.
    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let out = self.git(&["stash", "list", "--format=%ct%x1f%gs"])?;
        Ok(out
            .lines()
            .enumerate()
            .filter_map(|(index, l)| {
                let (time, message) = l.split_once('\u{1f}')?;
                Some(Stash {
                    index,
                    message: message.to_owned(),
                    time: time.parse().ok()?,
                })
            })
            .collect())
    }

    /// Set the changes aside, untracked files included, and leave the working tree clean.
    pub fn stash_push(&self, message: &str) -> Result<()> {
        let message = message.trim();
        let mut args = vec!["stash", "push", "--include-untracked", "--quiet"];
        if !message.is_empty() {
            args.extend(["--message", message]);
        }
        self.git(&args).map(drop)
    }

    /// Bring a stash back and keep it in the list.
    pub fn stash_apply(&self, index: usize) -> Result<()> {
        self.git(&["stash", "apply", "--quiet", &stash_ref(index)])
            .map(drop)
    }

    /// Bring a stash back and remove it from the list.
    pub fn stash_pop(&self, index: usize) -> Result<()> {
        self.git(&["stash", "pop", "--quiet", &stash_ref(index)])
            .map(drop)
    }

    /// Throw a stash away.
    pub fn stash_drop(&self, index: usize) -> Result<()> {
        self.git(&["stash", "drop", "--quiet", &stash_ref(index)])
            .map(drop)
    }

    /// The patch a stash holds, to look at before applying it.
    pub fn stash_show(&self, index: usize) -> Result<String> {
        self.git(&[
            "stash",
            "show",
            "--patch",
            "--include-untracked",
            "--no-color",
            &stash_ref(index),
        ])
        .map_err(|e| match e {
            Error::Failed(t) => Error::Failed(t),
            other => other,
        })
    }
}

fn stash_ref(index: usize) -> String {
    format!("stash@{{{index}}}")
}
