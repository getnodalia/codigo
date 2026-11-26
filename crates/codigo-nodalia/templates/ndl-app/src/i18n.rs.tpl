// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text in Spanish and English.
//!
//! The source is written in English, through [`tr`]; Spanish comes from [`SPANISH`], sorted by the
//! English text. A test reads every `tr("…")` in `src/` and fails when one has no Spanish entry,
//! so a new string cannot ship untranslated. Text with a value in it uses `{}` placeholders
//! filled by [`trf`].

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// The system language when it is Spanish, English otherwise.
    pub fn system() -> Lang {
        let tag = sys_locale::get_locale()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let spanish = tag
            .strip_prefix("es")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_', '.', '@']));
        if spanish {
            Lang::Es
        } else {
            Lang::En
        }
    }
}

static SPANISH_ON: AtomicBool = AtomicBool::new(false);

pub fn set(lang: Lang) {
    SPANISH_ON.store(lang == Lang::Es, Ordering::Relaxed);
}

/// The text in the current language.
pub fn tr(english: &'static str) -> &'static str {
    if !SPANISH_ON.load(Ordering::Relaxed) {
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
            assert_eq!(en.matches("{}").count(), es.matches("{}").count(), "{en}");
        }
    }

    #[test]
    fn trf_fills_placeholders_in_order() {
        set(Lang::En);
        assert_eq!(trf("Line {}, column {}", &["3", "9"]), "Line 3, column 9");
    }

    #[test]
    fn every_interface_string_has_spanish() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(src).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == "i18n.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
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
        assert!(
            missing.is_empty(),
            "strings without Spanish:\n{}",
            missing.join("\n")
        );
    }
}
