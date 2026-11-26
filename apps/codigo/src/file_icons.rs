// SPDX-License-Identifier: MIT OR Apache-2.0
//! One small icon per kind of file, so the explorer and the tabs tell files apart at a glance.
//! Each is a symbol drawn here as vector strokes (a gear, a bolt, a database, a flask...) inside a
//! rounded tile tinted from the palette. They are original to Código: no one else's logo.

use codigo_syntax::detect;
use egui::{pos2, CornerRadius, Painter, Rect, Shape, Stroke};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

/// Which palette tone a symbol takes.
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

type P = (f32, f32);

/// One stroke of a symbol, on a unit square.
#[derive(Clone, Copy)]
enum Op {
    /// An open polyline.
    Line(&'static [P]),
    /// A closed outline.
    Closed(&'static [P]),
    /// A filled convex polygon.
    Fill(&'static [P]),
    /// An elliptical arc: centre, radii, then start and end angles in degrees (0 is to the right,
    /// angles grow clockwise on screen).
    Arc(f32, f32, f32, f32, f32, f32),
    /// A filled dot: centre and radius.
    Dot(f32, f32, f32),
}

use Op::*;

const GEAR: &[Op] = &[
    Arc(0.5, 0.5, 0.2, 0.2, 0.0, 360.0),
    Dot(0.5, 0.5, 0.06),
    Line(&[(0.76, 0.5), (0.86, 0.5)]),
    Line(&[(0.24, 0.5), (0.14, 0.5)]),
    Line(&[(0.5, 0.76), (0.5, 0.86)]),
    Line(&[(0.5, 0.24), (0.5, 0.14)]),
    Line(&[(0.684, 0.684), (0.754, 0.754)]),
    Line(&[(0.316, 0.684), (0.246, 0.754)]),
    Line(&[(0.684, 0.316), (0.754, 0.246)]),
    Line(&[(0.316, 0.316), (0.246, 0.246)]),
];
const SNAKE: &[Op] = &[
    Line(&[
        (0.68, 0.28),
        (0.40, 0.28),
        (0.30, 0.38),
        (0.40, 0.48),
        (0.60, 0.52),
        (0.70, 0.62),
        (0.60, 0.72),
        (0.32, 0.72),
    ]),
    Dot(0.62, 0.28, 0.045),
];
const BOLT: &[Op] = &[Closed(&[
    (0.58, 0.14),
    (0.30, 0.54),
    (0.48, 0.54),
    (0.42, 0.86),
    (0.70, 0.44),
    (0.52, 0.44),
])];
const DIAMOND: &[Op] = &[
    Closed(&[(0.5, 0.15), (0.85, 0.5), (0.5, 0.85), (0.15, 0.5)]),
    Dot(0.5, 0.5, 0.07),
];
const ATOM: &[Op] = &[
    Closed(&[(0.5, 0.15), (0.85, 0.5), (0.5, 0.85), (0.15, 0.5)]),
    Arc(0.5, 0.5, 0.14, 0.14, 0.0, 360.0),
    Dot(0.5, 0.5, 0.045),
];
const BRACES: &[Op] = &[
    Line(&[
        (0.40, 0.2),
        (0.32, 0.24),
        (0.32, 0.42),
        (0.22, 0.5),
        (0.32, 0.58),
        (0.32, 0.76),
        (0.40, 0.8),
    ]),
    Line(&[
        (0.60, 0.2),
        (0.68, 0.24),
        (0.68, 0.42),
        (0.78, 0.5),
        (0.68, 0.58),
        (0.68, 0.76),
        (0.60, 0.8),
    ]),
];
const LIST: &[Op] = &[
    Dot(0.22, 0.3, 0.05),
    Line(&[(0.38, 0.3), (0.8, 0.3)]),
    Dot(0.22, 0.5, 0.05),
    Line(&[(0.38, 0.5), (0.8, 0.5)]),
    Dot(0.22, 0.7, 0.05),
    Line(&[(0.38, 0.7), (0.62, 0.7)]),
];
const TREE: &[Op] = &[
    Line(&[(0.2, 0.28), (0.8, 0.28)]),
    Line(&[(0.34, 0.5), (0.8, 0.5)]),
    Line(&[(0.48, 0.72), (0.8, 0.72)]),
    Line(&[(0.22, 0.34), (0.22, 0.5), (0.28, 0.5)]),
];
const MARKDOWN: &[Op] = &[
    Closed(&[(0.15, 0.28), (0.85, 0.28), (0.85, 0.72), (0.15, 0.72)]),
    Line(&[(0.5, 0.36), (0.5, 0.62)]),
    Line(&[(0.4, 0.52), (0.5, 0.63), (0.6, 0.52)]),
];
const SHELL: &[Op] = &[
    Line(&[(0.22, 0.3), (0.46, 0.5), (0.22, 0.7)]),
    Line(&[(0.52, 0.72), (0.8, 0.72)]),
];
const HEX: &[P] = &[
    (0.5, 0.14),
    (0.82, 0.32),
    (0.82, 0.68),
    (0.5, 0.86),
    (0.18, 0.68),
    (0.18, 0.32),
];
const C: &[Op] = &[Closed(HEX), Dot(0.5, 0.5, 0.06)];
const CPP: &[Op] = &[
    Closed(HEX),
    Line(&[(0.5, 0.4), (0.5, 0.6)]),
    Line(&[(0.4, 0.5), (0.6, 0.5)]),
];
const CSHARP: &[Op] = &[
    Closed(HEX),
    Line(&[(0.44, 0.38), (0.42, 0.62)]),
    Line(&[(0.58, 0.38), (0.56, 0.62)]),
    Line(&[(0.36, 0.45), (0.64, 0.44)]),
    Line(&[(0.35, 0.56), (0.63, 0.55)]),
];
const DASH: &[Op] = &[
    Line(&[(0.46, 0.5), (0.84, 0.5)]),
    Line(&[(0.7, 0.34), (0.84, 0.5), (0.7, 0.66)]),
    Line(&[(0.16, 0.36), (0.4, 0.36)]),
    Line(&[(0.16, 0.64), (0.4, 0.64)]),
];
const CUP: &[Op] = &[
    Closed(&[(0.22, 0.42), (0.64, 0.42), (0.6, 0.78), (0.26, 0.78)]),
    Line(&[(0.64, 0.5), (0.78, 0.5), (0.78, 0.64), (0.62, 0.66)]),
    Line(&[(0.36, 0.34), (0.32, 0.26), (0.38, 0.2)]),
    Line(&[(0.5, 0.34), (0.46, 0.26), (0.52, 0.2)]),
];
const TAGS: &[Op] = &[
    Line(&[(0.34, 0.3), (0.16, 0.5), (0.34, 0.7)]),
    Line(&[(0.66, 0.3), (0.84, 0.5), (0.66, 0.7)]),
    Line(&[(0.56, 0.24), (0.44, 0.76)]),
];
const DROP: &[Op] = &[
    Line(&[(0.31, 0.52), (0.5, 0.14), (0.69, 0.52)]),
    Arc(0.5, 0.64, 0.22, 0.22, -32.0, 212.0),
];
const DROP_DOT: &[Op] = &[
    Line(&[(0.31, 0.52), (0.5, 0.14), (0.69, 0.52)]),
    Arc(0.5, 0.64, 0.22, 0.22, -32.0, 212.0),
    Dot(0.5, 0.64, 0.055),
];
const ANGLES: &[Op] = &[
    Line(&[(0.4, 0.3), (0.2, 0.5), (0.4, 0.7)]),
    Line(&[(0.6, 0.3), (0.8, 0.5), (0.6, 0.7)]),
    Dot(0.5, 0.5, 0.05),
];
const DATABASE: &[Op] = &[
    Arc(0.5, 0.28, 0.3, 0.12, 0.0, 360.0),
    Line(&[(0.2, 0.28), (0.2, 0.72)]),
    Line(&[(0.8, 0.28), (0.8, 0.72)]),
    Arc(0.5, 0.5, 0.3, 0.12, 0.0, 180.0),
    Arc(0.5, 0.72, 0.3, 0.12, 0.0, 180.0),
];
const BOWTIE: &[Op] = &[Closed(&[
    (0.2, 0.2),
    (0.8, 0.2),
    (0.5, 0.5),
    (0.8, 0.8),
    (0.2, 0.8),
])];
const SWOOSH: &[Op] = &[
    Line(&[(0.18, 0.3), (0.4, 0.52), (0.62, 0.62), (0.82, 0.5)]),
    Line(&[(0.3, 0.68), (0.5, 0.76), (0.74, 0.76)]),
];
const TARGET: &[Op] = &[
    Arc(0.5, 0.5, 0.3, 0.3, 0.0, 360.0),
    Dot(0.5, 0.5, 0.06),
    Line(&[(0.5, 0.5), (0.84, 0.16)]),
    Line(&[(0.84, 0.3), (0.84, 0.16), (0.7, 0.16)]),
];
const LAYERS: &[Op] = &[
    Closed(&[(0.2, 0.34), (0.5, 0.2), (0.8, 0.34), (0.5, 0.48)]),
    Line(&[(0.2, 0.52), (0.5, 0.66), (0.8, 0.52)]),
    Line(&[(0.2, 0.68), (0.5, 0.82), (0.8, 0.68)]),
];
const WAVE: &[Op] = &[Line(&[
    (0.16, 0.62),
    (0.34, 0.38),
    (0.5, 0.62),
    (0.66, 0.38),
    (0.84, 0.62),
])];
const OVAL: &[Op] = &[
    Arc(0.5, 0.5, 0.34, 0.2, 0.0, 360.0),
    Dot(0.4, 0.5, 0.05),
    Dot(0.6, 0.5, 0.05),
];
const GEM: &[Op] = &[
    Closed(&[
        (0.3, 0.22),
        (0.7, 0.22),
        (0.86, 0.42),
        (0.5, 0.82),
        (0.14, 0.42),
    ]),
    Line(&[(0.14, 0.42), (0.86, 0.42)]),
    Line(&[(0.38, 0.42), (0.5, 0.82), (0.62, 0.42)]),
];
const MOON: &[Op] = &[
    Arc(0.5, 0.5, 0.32, 0.32, 40.0, 320.0),
    Dot(0.72, 0.32, 0.07),
];
const FLASK: &[Op] = &[
    Line(&[
        (0.42, 0.16),
        (0.42, 0.42),
        (0.2, 0.78),
        (0.8, 0.78),
        (0.58, 0.42),
        (0.58, 0.16),
    ]),
    Line(&[(0.36, 0.16), (0.64, 0.16)]),
    Dot(0.5, 0.64, 0.05),
];
const LAMBDA: &[Op] = &[
    Line(&[(0.26, 0.2), (0.7, 0.8)]),
    Line(&[(0.47, 0.48), (0.26, 0.8)]),
];
const HUMPS: &[Op] = &[
    Arc(0.35, 0.62, 0.16, 0.2, 180.0, 360.0),
    Arc(0.65, 0.62, 0.16, 0.2, 180.0, 360.0),
    Line(&[(0.19, 0.62), (0.19, 0.8)]),
    Line(&[(0.5, 0.62), (0.5, 0.8)]),
    Line(&[(0.81, 0.62), (0.81, 0.8)]),
];
const PEARL: &[Op] = &[Arc(0.5, 0.5, 0.3, 0.3, 0.0, 360.0), Dot(0.4, 0.4, 0.05)];
const BARS: &[Op] = &[
    Fill(&[(0.2, 0.56), (0.34, 0.56), (0.34, 0.8), (0.2, 0.8)]),
    Fill(&[(0.43, 0.3), (0.57, 0.3), (0.57, 0.8), (0.43, 0.8)]),
    Fill(&[(0.66, 0.44), (0.8, 0.44), (0.8, 0.8), (0.66, 0.8)]),
];
const CONTAINERS: &[Op] = &[
    Closed(&[(0.14, 0.52), (0.36, 0.52), (0.36, 0.72), (0.14, 0.72)]),
    Closed(&[(0.39, 0.52), (0.61, 0.52), (0.61, 0.72), (0.39, 0.72)]),
    Closed(&[(0.64, 0.52), (0.86, 0.52), (0.86, 0.72), (0.64, 0.72)]),
    Closed(&[(0.26, 0.29), (0.48, 0.29), (0.48, 0.49), (0.26, 0.49)]),
    Closed(&[(0.52, 0.29), (0.74, 0.29), (0.74, 0.49), (0.52, 0.49)]),
];
const HAMMER: &[Op] = &[
    Line(&[(0.22, 0.8), (0.58, 0.46)]),
    Fill(&[(0.44, 0.28), (0.7, 0.18), (0.84, 0.34), (0.58, 0.5)]),
];
const TRIANGLES: &[Op] = &[
    Closed(&[(0.5, 0.16), (0.86, 0.82), (0.14, 0.82)]),
    Closed(&[(0.5, 0.46), (0.66, 0.72), (0.34, 0.72)]),
];
const SLIDERS: &[Op] = &[
    Line(&[(0.16, 0.32), (0.84, 0.32)]),
    Dot(0.38, 0.32, 0.08),
    Line(&[(0.16, 0.68), (0.84, 0.68)]),
    Dot(0.66, 0.68, 0.08),
];
const PICTURE: &[Op] = &[
    Closed(&[(0.16, 0.24), (0.84, 0.24), (0.84, 0.76), (0.16, 0.76)]),
    Line(&[
        (0.16, 0.66),
        (0.38, 0.46),
        (0.54, 0.62),
        (0.64, 0.54),
        (0.84, 0.7),
    ]),
    Dot(0.68, 0.38, 0.05),
];
const BRANCH: &[Op] = &[
    Line(&[(0.3, 0.26), (0.3, 0.74)]),
    Line(&[(0.3, 0.6), (0.5, 0.5), (0.7, 0.38)]),
    Dot(0.3, 0.22, 0.07),
    Dot(0.3, 0.78, 0.07),
    Dot(0.72, 0.34, 0.07),
];
const PADLOCK: &[Op] = &[
    Closed(&[(0.22, 0.46), (0.78, 0.46), (0.78, 0.84), (0.22, 0.84)]),
    Arc(0.5, 0.46, 0.16, 0.2, 180.0, 360.0),
    Dot(0.5, 0.65, 0.05),
];

/// The symbol and tone of a language id, or `None` for plain text (which keeps the generic icon).
fn language_glyph(id: &str) -> Option<(&'static [Op], Tone)> {
    use Tone::*;
    Some(match id {
        "rust" => (GEAR, Orange),
        "python" => (SNAKE, Blue),
        "javascript" => (BOLT, Amber),
        "typescript" => (DIAMOND, Blue),
        "tsx" => (ATOM, Blue),
        "json" => (BRACES, Amber),
        "toml" => (LIST, Red),
        "yaml" => (TREE, Red),
        "markdown" => (MARKDOWN, Quiet),
        "bash" => (SHELL, Green),
        "c" => (C, Blue),
        "cpp" => (CPP, Blue),
        "go" => (DASH, Accent),
        "java" => (CUP, Red),
        "html" => (TAGS, Orange),
        "css" => (DROP, Blue),
        "scss" => (DROP_DOT, Violet),
        "xml" => (ANGLES, Orange),
        "sql" => (DATABASE, Violet),
        "csharp" => (CSHARP, Violet),
        "kotlin" => (BOWTIE, Violet),
        "swift" => (SWOOSH, Orange),
        "dart" => (TARGET, Accent),
        "scala" => (LAYERS, Red),
        "zig" => (WAVE, Amber),
        "php" => (OVAL, Violet),
        "ruby" => (GEM, Red),
        "lua" => (MOON, Blue),
        "elixir" => (FLASK, Violet),
        "haskell" => (LAMBDA, Violet),
        "ocaml" => (HUMPS, Orange),
        "perl" => (PEARL, Blue),
        "r" => (BARS, Blue),
        "dockerfile" => (CONTAINERS, Blue),
        "makefile" => (HAMMER, Quiet),
        "cmake" => (TRIANGLES, Green),
        "ini" => (SLIDERS, Quiet),
        _ => return None,
    })
}

/// Kinds of file that are not source languages.
fn other_glyph(name: &str) -> Option<(&'static [Op], Tone)> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with(".git") {
        return Some((BRANCH, Tone::Orange));
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e)?;
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "avif" => {
            Some((PICTURE, Tone::Green))
        }
        "lock" => Some((PADLOCK, Tone::Quiet)),
        _ => None,
    }
}

fn glyph(name: &str) -> Option<(&'static [Op], Tone)> {
    other_glyph(name).or_else(|| language_glyph(detect(name).id))
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

/// Whether `file_name` has a symbol of its own.
pub fn has(file_name: &str) -> bool {
    glyph(file_name).is_some()
}

fn arc_points(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<P> {
    let steps = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Draw the icon of `file_name` inside `rect`. Returns false for a file with no symbol of its
/// own, so the caller can draw its generic file icon.
pub fn paint(painter: &Painter, look: &Look, rect: Rect, file_name: &str) -> bool {
    let Some((ops, tone)) = glyph(file_name) else {
        return false;
    };
    let ink = tone_color(look, tone);
    painter.rect_filled(rect, CornerRadius::same(4), ink.gamma_multiply(0.2));
    let size = rect.width();
    let at = |&(x, y): &P| pos2(rect.left() + x * size, rect.top() + y * size);
    let stroke = Stroke::new((size * 0.085).max(1.0), ink);
    for op in ops {
        match *op {
            Line(points) => {
                painter.add(Shape::line(points.iter().map(at).collect(), stroke));
            }
            Closed(points) => {
                painter.add(Shape::closed_line(points.iter().map(at).collect(), stroke));
            }
            Fill(points) => {
                painter.add(Shape::convex_polygon(
                    points.iter().map(at).collect(),
                    ink,
                    Stroke::NONE,
                ));
            }
            Arc(cx, cy, rx, ry, a0, a1) => {
                let points: Vec<_> = arc_points(cx, cy, rx, ry, a0, a1).iter().map(at).collect();
                if (a1 - a0).abs() >= 360.0 {
                    painter.add(Shape::closed_line(points, stroke));
                } else {
                    painter.add(Shape::line(points, stroke));
                }
            }
            Dot(cx, cy, r) => {
                painter.circle_filled(at(&(cx, cy)), r * size, ink);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(p: &P) -> bool {
        (0.0..=1.0).contains(&p.0) && (0.0..=1.0).contains(&p.1)
    }

    fn check(ops: &[Op]) {
        assert!(!ops.is_empty());
        for op in ops {
            match op {
                Line(p) | Closed(p) | Fill(p) => assert!(p.len() >= 2 && p.iter().all(inside)),
                Arc(cx, cy, rx, ry, a0, a1) => {
                    assert!(arc_points(*cx, *cy, *rx, *ry, *a0, *a1).iter().all(inside))
                }
                Dot(cx, cy, r) => assert!(inside(&(cx - r, cy - r)) && inside(&(cx + r, cy + r))),
            }
        }
    }

    #[test]
    fn every_language_has_a_symbol_except_plain_text() {
        for lang in codigo_syntax::languages() {
            assert_eq!(
                language_glyph(lang.id).is_some(),
                lang.id != "text",
                "language {} needs a file icon",
                lang.id
            );
        }
    }

    #[test]
    fn every_symbol_stays_inside_its_tile() {
        for lang in codigo_syntax::languages() {
            if let Some((ops, _)) = language_glyph(lang.id) {
                check(ops);
            }
        }
        for name in ["a.png", "Cargo.lock", ".gitignore"] {
            check(other_glyph(name).map(|g| g.0).unwrap_or(&[]));
        }
    }

    #[test]
    fn files_are_recognised_by_name() {
        assert!(has("main.rs"));
        assert!(has("app.JS"));
        assert!(has("Dockerfile"));
        assert!(has("photo.PNG"));
        assert!(has(".gitignore"));
        assert!(!has("notes.txt"));
        assert!(!has("noextension"));
    }
}
