// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text in Spanish and English.
//!
//! The source is written in English, through [`tr`]; Spanish comes from [`SPANISH`]. A test reads
//! every `tr("…")` in `src/` and every command label and fails when one has no Spanish entry, so a
//! new string cannot ship untranslated. Text with a value in it uses `{}` placeholders filled by
//! [`trf`].

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// The system language when it is Spanish, English otherwise. macOS and Windows keep the
    /// language in their own settings (an app opened from the Dock has no `LANG`), so ask them;
    /// Linux and the BSDs say it in the environment.
    pub fn system() -> Lang {
        if system_tag().is_some_and(|tag| is_spanish(&tag)) {
            Lang::Es
        } else {
            Lang::En
        }
    }

    /// The language a settings value names: `es`, `en`, or `auto` for the system's.
    pub fn from_setting(value: &str) -> Lang {
        match value {
            "es" => Lang::Es,
            "en" => Lang::En,
            _ => Lang::system(),
        }
    }
}

fn system_tag() -> Option<String> {
    let platform = || sys_locale::get_locale().filter(|tag| !tag.is_empty() && tag.len() <= 64);
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return platform();
    }
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .or_else(platform)
}

/// Whether a locale tag (`es`, `es-ES`, `es_MX.UTF-8`, `es-419`) names Spanish.
fn is_spanish(tag: &str) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    tag.strip_prefix("es")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_', '.', '@']))
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Es => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::Es,
        _ => Lang::En,
    }
}

/// The text in the current language.
pub fn tr(english: &'static str) -> &'static str {
    if current() == Lang::En {
        return english;
    }
    SPANISH
        .binary_search_by(|(en, _)| en.cmp(&english))
        .ok()
        .and_then(|i| SPANISH.get(i))
        .map_or(english, |(_, es)| es)
}

/// [`tr`] with each `{}` replaced, in order, by `args`.
pub fn trf(english: &'static str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = tr(english);
    for arg in args {
        match rest.split_once("{}") {
            Some((before, after)) => {
                out.push_str(before);
                out.push_str(arg);
                rest = after;
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// English → Spanish, sorted by the English text (the lookup is a binary search).
pub use crate::spanish::SPANISH;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        for pair in SPANISH.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{:?} is out of order", pair[1].0);
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        for (en, es) in SPANISH {
            assert_eq!(
                en.matches("{}").count(),
                es.matches("{}").count(),
                "{en}: placeholders differ"
            );
        }
    }

    #[test]
    fn trf_fills_placeholders_in_order() {
        set(Lang::En);
        assert_eq!(trf("Line {}, column {}", &["3", "9"]), "Line 3, column 9");
        set(Lang::Es);
    }

    #[test]
    fn spanish_tags() {
        for tag in ["es", "es-ES", "es_MX.UTF-8", "es-419", "ES"] {
            assert!(is_spanish(tag), "{tag}");
        }
        for tag in ["en", "en_US", "eu", "est", ""] {
            assert!(!is_spanish(tag), "{tag}");
        }
    }

    #[test]
    fn every_quality_rule_has_spanish() {
        for rule in codigo_quality::RULES {
            for text in [rule.title, rule.advice] {
                assert!(
                    SPANISH.binary_search_by_key(&text, |(en, _)| en).is_ok(),
                    "{}: {text}",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn every_command_and_category_has_spanish() {
        let has = |english: &str| SPANISH.binary_search_by(|(en, _)| en.cmp(&english)).is_ok();
        let mut missing: Vec<String> = codigo_engine::Cmd::ALL
            .iter()
            .filter(|c| !has(c.label()))
            .map(|c| format!("command {}: {}", c.id(), c.label()))
            .collect();
        for cat in [
            codigo_engine::Category::File,
            codigo_engine::Category::Edit,
            codigo_engine::Category::Selection,
            codigo_engine::Category::View,
            codigo_engine::Category::Go,
            codigo_engine::Category::Run,
            codigo_engine::Category::Git,
            codigo_engine::Category::GitHub,
            codigo_engine::Category::GitLab,
            codigo_engine::Category::Nodalia,
            codigo_engine::Category::Help,
        ] {
            if !has(cat.label()) {
                missing.push(format!("category: {}", cat.label()));
            }
        }
        assert!(
            missing.is_empty(),
            "without Spanish:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn every_interface_string_has_spanish() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read file");
                for marker in ["tr(", "trf("] {
                    for (at, _) in text.match_indices(marker) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = text[at + marker.len()..].trim_start();
                        let Some(piece) = rest.strip_prefix('"') else {
                            continue;
                        };
                        let Some(end) = piece.find('"') else {
                            continue;
                        };
                        let english = &piece[..end];
                        if SPANISH
                            .binary_search_by(|(en, _)| en.cmp(&english))
                            .is_err()
                        {
                            missing.push(format!("{}: {english}", path.display()));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "strings without Spanish:\n{}",
            missing.join("\n")
        );
    }
}
