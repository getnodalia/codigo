// SPDX-License-Identifier: MIT OR Apache-2.0
//! Who last changed each line of a file.

use std::collections::HashMap;

use crate::{Repo, Result};

/// The commit behind one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Seven characters of the hash; all zeros for a line not committed yet.
    pub short: String,
    pub author: String,
    /// Seconds since the epoch.
    pub time: i64,
    pub summary: String,
}

impl BlameLine {
    /// A line that is in the working tree but not in any commit.
    pub fn uncommitted(&self) -> bool {
        self.short.chars().all(|c| c == '0')
    }
}

impl Repo {
    /// One entry per line of `path` (relative to the root), or an error for a file git does not
    /// track.
    pub fn blame(&self, path: &str) -> Result<Vec<BlameLine>> {
        let out = self.git(&["blame", "--line-porcelain", "--", path])?;
        Ok(parse(&out))
    }
}

/// Reads `git blame --line-porcelain`: every line repeats its commit's header.
pub fn parse(text: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    let mut known: HashMap<String, (String, i64, String)> = HashMap::new();
    let mut current: Option<(String, String, i64, String)> = None;
    for raw in text.lines() {
        if let Some(content) = raw.strip_prefix('\t') {
            let _ = content;
            if let Some((hash, author, time, summary)) = current.take() {
                known.insert(hash.clone(), (author.clone(), time, summary.clone()));
                lines.push(BlameLine {
                    short: hash.chars().take(7).collect(),
                    author,
                    time,
                    summary,
                });
            }
        } else if let Some(v) = raw.strip_prefix("author ") {
            if let Some(c) = current.as_mut() {
                c.1 = v.to_owned();
            }
        } else if let Some(v) = raw.strip_prefix("author-time ") {
            if let Some(c) = current.as_mut() {
                c.2 = v.trim().parse().unwrap_or(0);
            }
        } else if let Some(v) = raw.strip_prefix("summary ") {
            if let Some(c) = current.as_mut() {
                c.3 = v.to_owned();
            }
        } else if let Some(hash) = header_hash(raw) {
            let (author, time, summary) = known.get(&hash).cloned().unwrap_or_default();
            current = Some((hash, author, time, summary));
        }
    }
    lines
}

/// `<40 hex> <orig line> <final line> [<group size>]` starts each entry.
fn header_hash(line: &str) -> Option<String> {
    let mut parts = line.split(' ');
    let hash = parts.next()?;
    let ok = hash.len() >= 40 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    (ok && parts.next().is_some_and(|n| n.parse::<u64>().is_ok())).then(|| hash.to_owned())
}
