// SPDX-License-Identifier: MIT OR Apache-2.0
//! The user's preferences, saved as JSON.

use std::collections::BTreeMap;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Line,
    Block,
    Underline,
}

pub const MIN_LINE_HEIGHT: f32 = 1.1;
pub const MAX_LINE_HEIGHT: f32 = 2.4;
pub const MIN_UI_SCALE: f32 = 0.75;
pub const MAX_UI_SCALE: f32 = 2.0;
/// How many ruler columns, and how far right, a settings file may ask for.
const MAX_RULERS: usize = 8;
const MAX_RULER_COLUMN: u16 = 500;
const MAX_OVERRIDES: usize = 64;
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
    /// The code font's family name; empty for the built-in one.
    pub font_family: String,
    /// Row height as a multiple of the font size.
    pub line_height: f32,
    /// The id of a colour theme (`nodalia` follows the window's own colours).
    pub color_theme: String,
    /// Colours the person changed on top of the theme, keyed by slot (`syntax.keyword`) and
    /// written as `#rrggbb`.
    pub color_overrides: BTreeMap<String, String>,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// Columns where a thin vertical line is drawn.
    pub rulers: Vec<u16>,
    pub indent_guides: bool,
    /// How much bigger or smaller the whole window is drawn.
    pub ui_scale: f32,
    /// The program the terminal starts; empty for the login shell.
    pub terminal_shell: String,
    /// The GitLab server the GitLab panel talks to (`gitlab.com` or a self-hosted one).
    pub gitlab_host: String,
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
    /// Show who last changed the line the cursor is on, at the end of the line.
    pub blame: bool,
    /// The assistant's provider (`anthropic`, `openai`, `ollama` or `custom`). Never a key: those
    /// live in the system's keyring.
    pub ai_provider: String,
    /// The model to ask; empty for the provider's usual one.
    pub ai_model: String,
    /// Where the provider is; empty for its usual address.
    pub ai_base_url: String,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            language: "auto".into(),
            font_size: 14.0,
            font_family: String::new(),
            line_height: 1.55,
            color_theme: crate::theme::NODALIA.into(),
            color_overrides: BTreeMap::new(),
            cursor_style: CursorStyle::Line,
            cursor_blink: true,
            rulers: Vec::new(),
            indent_guides: false,
            ui_scale: 1.0,
            terminal_shell: String::new(),
            gitlab_host: "gitlab.com".to_owned(),
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
            blame: false,
            ai_provider: "anthropic".into(),
            ai_model: String::new(),
            ai_base_url: String::new(),
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
        let clamp = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        self.line_height = clamp(self.line_height, MIN_LINE_HEIGHT, MAX_LINE_HEIGHT, 1.55);
        self.ui_scale = clamp(self.ui_scale, MIN_UI_SCALE, MAX_UI_SCALE, 1.0);
        self.font_family = self
            .font_family
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect::<String>()
            .trim()
            .to_owned();
        self.terminal_shell = self.terminal_shell.trim().chars().take(500).collect();
        self.gitlab_host = self.gitlab_host.trim().to_ascii_lowercase();
        if self.gitlab_host.is_empty() || self.gitlab_host.len() > 253 {
            self.gitlab_host = "gitlab.com".to_owned();
        }
        if self.color_theme != crate::theme::NODALIA
            && crate::theme::preset(&self.color_theme).is_none()
        {
            self.color_theme = crate::theme::NODALIA.into();
        }
        self.color_overrides.retain(|k, v| {
            crate::theme::Slot::from_key(k).is_some() && crate::theme::Rgb::parse(v).is_some()
        });
        while self.color_overrides.len() > MAX_OVERRIDES {
            self.color_overrides.pop_last();
        }
        self.rulers.retain(|c| (1..=MAX_RULER_COLUMN).contains(c));
        self.rulers.sort_unstable();
        self.rulers.dedup();
        self.rulers.truncate(MAX_RULERS);
        self.auto_save_delay_ms = self.auto_save_delay_ms.clamp(200, 60_000);
        if self.language.trim().is_empty() {
            self.language = "auto".into();
        }
        self.recent_projects.truncate(MAX_RECENT);
        self.ai_model.truncate(200);
        self.ai_base_url.truncate(500);
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
    fn appearance_values_are_repaired() {
        let mut s = Settings {
            line_height: f32::INFINITY,
            ui_scale: 9.0,
            font_family: "  Fira\u{0}Code \n".into(),
            color_theme: "no-such-theme".into(),
            rulers: vec![120, 0, 80, 80, 9999, 100],
            ..Settings::default()
        };
        s.color_overrides
            .insert("syntax.keyword".into(), "#ff0000".into());
        s.color_overrides
            .insert("syntax.keyword2".into(), "#00ff00".into());
        s.color_overrides
            .insert("editor.text".into(), "banana".into());
        s.sanitize();
        assert_eq!(s.line_height, 1.55);
        assert_eq!(s.ui_scale, MAX_UI_SCALE);
        assert_eq!(s.font_family, "FiraCode");
        assert_eq!(s.color_theme, "nodalia");
        assert_eq!(s.rulers, vec![80, 100, 120]);
        assert_eq!(s.color_overrides.len(), 1);
        assert!(s.color_overrides.contains_key("syntax.keyword"));
    }

    #[test]
    fn a_known_theme_and_its_overrides_survive_a_round_trip() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("s.json");
        let mut s = Settings {
            color_theme: "dracula".into(),
            cursor_style: CursorStyle::Block,
            rulers: vec![100],
            ..Settings::default()
        };
        s.color_overrides
            .insert("editor.background".into(), "#101010".into());
        s.save_to(&p).expect("save");
        assert_eq!(Settings::load_from(&p), s);
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
