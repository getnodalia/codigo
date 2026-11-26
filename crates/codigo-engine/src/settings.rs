// SPDX-License-Identifier: MIT OR Apache-2.0
//! The user's preferences, saved as JSON.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::config_dir;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoSave {
    #[default]
    Off,
    /// When the window loses focus.
    OnFocusLoss,
    /// A moment after the last edit.
    AfterDelay,
}

pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 40.0;
const MAX_RECENT: usize = 12;

/// Every field has a default, so a settings file from an older or newer version still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    /// `auto` follows the system; otherwise a language code such as `es` or `en`.
    pub language: String,
    pub font_size: f32,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub highlight_active_line: bool,
    pub show_whitespace: bool,
    /// The overview strip beside the editor.
    pub minimap: bool,
    pub auto_close_pairs: bool,
    pub trim_trailing_whitespace_on_save: bool,
    pub insert_final_newline_on_save: bool,
    pub auto_save: AutoSave,
    /// Milliseconds before `AutoSave::AfterDelay` saves.
    pub auto_save_delay_ms: u64,
    /// Linux only: draw the desktop's own window controls instead of Nodalia's.
    pub native_window_controls: bool,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    pub check_updates: bool,
    /// Start language servers for the languages that have one on this computer.
    pub language_servers: bool,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            language: "auto".into(),
            font_size: 14.0,
            word_wrap: false,
            line_numbers: true,
            highlight_active_line: true,
            show_whitespace: false,
            minimap: true,
            auto_close_pairs: true,
            trim_trailing_whitespace_on_save: false,
            insert_final_newline_on_save: false,
            auto_save: AutoSave::Off,
            auto_save_delay_ms: 1000,
            native_window_controls: false,
            high_contrast: false,
            reduced_motion: false,
            check_updates: true,
            language_servers: true,
            recent_projects: Vec::new(),
        }
    }
}

impl Settings {
    /// Bring every value into its legal range. Called after loading, so a hand-edited file
    /// cannot ask for a 400-point font.
    pub fn sanitize(&mut self) {
        self.font_size = if self.font_size.is_finite() {
            self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
        } else {
            14.0
        };
        self.auto_save_delay_ms = self.auto_save_delay_ms.clamp(200, 60_000);
        if self.language.trim().is_empty() {
            self.language = "auto".into();
        }
        self.recent_projects.truncate(MAX_RECENT);
    }

    /// Put a project at the front of the recent list.
    pub fn remember_project(&mut self, path: &Path) {
        self.recent_projects.retain(|p| p != path);
        self.recent_projects.insert(0, path.to_path_buf());
        self.recent_projects.truncate(MAX_RECENT);
    }

    pub fn file() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    /// Read the settings file. A missing file gives the defaults; a file that cannot be parsed is
    /// moved aside to `settings.json.bad` and the defaults are used, so one stray comma never
    /// stops the editor from starting.
    pub fn load_from(path: &Path) -> Settings {
        let Ok(text) = fs::read_to_string(path) else {
            return Settings::default();
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.sanitize();
                s
            }
            Err(_) => {
                let mut bad = path.as_os_str().to_owned();
                bad.push(".bad");
                let _ = fs::rename(path, bad);
                Settings::default()
            }
        }
    }

    pub fn load() -> Settings {
        Settings::file().map_or_else(Settings::default, |p| Settings::load_from(&p))
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
        match Settings::file() {
            Some(p) => self.save_to(&p),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_missing_file_gives_defaults() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("sub/settings.json");
        assert_eq!(Settings::load_from(&p), Settings::default());
        let mut s = Settings {
            appearance: Appearance::Dark,
            font_size: 17.0,
            auto_save: AutoSave::AfterDelay,
            ..Settings::default()
        };
        s.remember_project(Path::new("/a"));
        s.save_to(&p).expect("save");
        assert_eq!(Settings::load_from(&p), s);
    }

    #[test]
    fn old_and_partial_files_load_and_values_are_clamped() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("s.json");
        fs::write(
            &p,
            r#"{"font_size": 400, "word_wrap": true, "future_option": 1}"#,
        )
        .expect("w");
        let s = Settings::load_from(&p);
        assert_eq!(s.font_size, MAX_FONT_SIZE);
        assert!(s.word_wrap && s.line_numbers);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_defaults_used() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("s.json");
        fs::write(&p, "{ not json").expect("w");
        assert_eq!(Settings::load_from(&p), Settings::default());
        assert!(!p.exists());
        assert!(d.path().join("s.json.bad").exists());
    }

    #[test]
    fn recent_projects_are_unique_newest_first_and_capped() {
        let mut s = Settings::default();
        for i in 0..20 {
            s.remember_project(Path::new(&format!("/p{i}")));
        }
        s.remember_project(Path::new("/p5"));
        assert_eq!(s.recent_projects.len(), MAX_RECENT);
        assert_eq!(s.recent_projects[0], Path::new("/p5"));
        assert_eq!(
            s.recent_projects
                .iter()
                .filter(|p| **p == Path::new("/p5"))
                .count(),
            1
        );
    }

    #[test]
    fn nan_and_empty_values_are_repaired() {
        let mut s = Settings {
            font_size: f32::NAN,
            language: "  ".into(),
            auto_save_delay_ms: 0,
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(
            (s.font_size, s.language.as_str(), s.auto_save_delay_ms),
            (14.0, "auto", 200)
        );
    }
}
