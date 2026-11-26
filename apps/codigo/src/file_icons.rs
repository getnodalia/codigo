// SPDX-License-Identifier: MIT OR Apache-2.0
//! One small icon per kind of file, so the explorer and the tabs tell files apart at a glance.
//! Each is a rounded tile with the language's short mark, drawn here and tinted from the palette:
//! original to Código, no one else's logo.

use codigo_syntax::detect;
use egui::{pos2, Align2, CornerRadius, FontId, Painter, Rect};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

/// Which palette tone a mark takes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tone {
    Accent,
    Blue,
    Green,
    Amber,
    Red,
    Orange,
    Violet,
    Quiet,
}

/// The mark and tone of a language id, or `None` for plain text (which keeps the generic icon).
fn badge(id: &str) -> Option<(&'static str, Tone)> {
    use Tone::*;
    Some(match id {
        "rust" => ("RS", Orange),
        "python" => ("PY", Blue),
        "javascript" => ("JS", Amber),
        "typescript" => ("TS", Blue),
        "tsx" => ("TX", Blue),
        "json" => ("{}", Amber),
        "toml" => ("TM", Red),
        "yaml" => ("YM", Red),
        "markdown" => ("MD", Quiet),
        "bash" => ("SH", Green),
        "c" => ("C", Blue),
        "cpp" => ("C+", Blue),
        "go" => ("GO", Accent),
        "java" => ("JV", Red),
        "html" => ("<>", Orange),
        "css" => ("CS", Blue),
        "scss" => ("SC", Violet),
        "xml" => ("XM", Orange),
        "sql" => ("SQ", Violet),
        "csharp" => ("C#", Violet),
        "kotlin" => ("KT", Violet),
        "swift" => ("SW", Orange),
        "dart" => ("DT", Accent),
        "scala" => ("SL", Red),
        "zig" => ("ZG", Amber),
        "php" => ("PH", Violet),
        "ruby" => ("RB", Red),
        "lua" => ("LU", Blue),
        "elixir" => ("EX", Violet),
        "haskell" => ("HS", Violet),
        "ocaml" => ("ML", Orange),
        "perl" => ("PL", Blue),
        "r" => ("R", Blue),
        "dockerfile" => ("DK", Blue),
        "makefile" => ("MK", Quiet),
        "cmake" => ("CM", Green),
        "ini" => ("IN", Quiet),
        _ => return None,
    })
}

fn tone_color(look: &Look, tone: Tone) -> egui::Color32 {
    let p = &look.palette;
    let amber = color(p.status_warning_fg);
    let red = color(p.status_danger_fg);
    let blue = color(p.status_info_fg);
    let accent = color(p.accent_text);
    match tone {
        Tone::Accent => accent,
        Tone::Blue => blue,
        Tone::Green => color(p.status_success_fg),
        Tone::Amber => amber,
        Tone::Red => red,
        Tone::Orange => mix(red, amber, 0.5),
        Tone::Violet => mix(accent, red, 0.45),
        Tone::Quiet => color(p.text_secondary),
    }
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// Whether `file_name` has a mark of its own.
pub fn has(file_name: &str) -> bool {
    badge(detect(file_name).id).is_some()
}

/// Draw the icon of `file_name` inside `rect`. Returns false for a file with no mark of its own,
/// so the caller can draw its generic file icon.
pub fn paint(painter: &Painter, look: &Look, rect: Rect, file_name: &str) -> bool {
    let Some((mark, tone)) = badge(detect(file_name).id) else {
        return false;
    };
    let ink = tone_color(look, tone);
    painter.rect_filled(rect, CornerRadius::same(4), ink.gamma_multiply(0.2));
    let size = if mark.chars().count() > 1 { 8.5 } else { 10.0 };
    painter.text(
        pos2(rect.center().x, rect.center().y + 0.3),
        Align2::CENTER_CENTER,
        mark,
        FontId::monospace(size),
        ink,
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_a_mark_except_plain_text() {
        for lang in codigo_syntax::languages() {
            assert_eq!(
                badge(lang.id).is_some(),
                lang.id != "text",
                "language {} needs a file icon",
                lang.id
            );
        }
    }

    #[test]
    fn marks_are_short() {
        for lang in codigo_syntax::languages() {
            if let Some((mark, _)) = badge(lang.id) {
                assert!(mark.chars().count() <= 2, "{mark}");
            }
        }
    }

    #[test]
    fn files_are_recognised_by_name() {
        assert_eq!(badge(detect("main.rs").id).map(|b| b.0), Some("RS"));
        assert_eq!(badge(detect("app.JS").id).map(|b| b.0), Some("JS"));
        assert_eq!(badge(detect("Dockerfile").id).map(|b| b.0), Some("DK"));
        assert!(badge(detect("notes.txt").id).is_none());
    }
}
