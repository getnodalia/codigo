// SPDX-License-Identifier: MIT OR Apache-2.0
//! What was open when the editor last closed, so the next launch can pick up where it left off.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::config_dir;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub project: Option<PathBuf>,
    /// Files open as tabs, in order. Documents that never had a file are not remembered.
    pub tabs: Vec<PathBuf>,
    pub active: usize,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
}

impl SessionState {
    pub fn file() -> Option<PathBuf> {
        config_dir().map(|d| d.join("session.json"))
    }

    /// Read the saved session. Anything unreadable means a fresh start, never an error.
    pub fn load_from(path: &Path) -> SessionState {
        let mut s = fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<SessionState>(&t).ok())
            .unwrap_or_default();
        s.sanitize();
        s
    }

    pub fn load() -> SessionState {
        SessionState::file().map_or_else(SessionState::default, |p| SessionState::load_from(&p))
    }

    /// Drop what no longer exists and keep the numbers in range.
    pub fn sanitize(&mut self) {
        if self.project.as_ref().is_some_and(|p| !p.is_dir()) {
            self.project = None;
        }
        self.tabs.retain(|p| p.is_file());
        self.tabs.truncate(64);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.sidebar_width = if self.sidebar_width.is_finite() {
            self.sidebar_width.clamp(160.0, 640.0)
        } else {
            260.0
        };
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)
    }

    pub fn save(&self) -> io::Result<()> {
        match SessionState::file() {
            Some(p) => self.save_to(&p),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_are_dropped_and_indexes_stay_valid() {
        let d = tempfile::tempdir().expect("tempdir");
        let keep = d.path().join("keep.rs");
        fs::write(&keep, "").expect("keep");
        let s = SessionState {
            project: Some(d.path().join("gone")),
            tabs: vec![d.path().join("gone.rs"), keep.clone()],
            active: 9,
            sidebar_visible: true,
            sidebar_width: 9999.0,
        };
        let p = d.path().join("session.json");
        s.save_to(&p).expect("save");
        let back = SessionState::load_from(&p);
        assert_eq!(back.project, None);
        assert_eq!(back.tabs, vec![keep]);
        assert_eq!(back.active, 0);
        assert_eq!(back.sidebar_width, 640.0);
    }

    #[test]
    fn a_broken_file_is_a_fresh_start() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("session.json");
        fs::write(&p, "][").expect("w");
        assert_eq!(SessionState::load_from(&p).tabs.len(), 0);
        assert_eq!(SessionState::load_from(&d.path().join("none")).active, 0);
    }
}
