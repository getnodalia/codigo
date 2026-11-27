// SPDX-License-Identifier: MIT OR Apache-2.0
//! Editor colour themes: the colours of the text area and of each kind of code, the built-in
//! presets, and the per-colour overrides a person keeps in their settings.
//!
//! This module only knows about colours. A frontend turns a [`Colors`] into pixels, and the
//! window's own chrome keeps following the Nodalia palette.

use std::collections::BTreeMap;

use codigo_syntax::Scope;

/// An opaque sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `#rrggbb` or `#rgb`, with or without the `#`. Anything else is `None`.
    pub fn parse(text: &str) -> Option<Rgb> {
        let t = text.trim().trim_start_matches('#');
        if !t.is_ascii() {
            return None;
        }
        let hex = |s: &str| u8::from_str_radix(s, 16).ok();
        match t.len() {
            6 => Some(Rgb(hex(&t[0..2])?, hex(&t[2..4])?, hex(&t[4..6])?)),
            3 => {
                let d = |i: usize| hex(&t[i..=i]).map(|v| v * 17);
                Some(Rgb(d(0)?, d(1)?, d(2)?))
            }
            _ => None,
        }
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    const fn from_u24(v: u32) -> Rgb {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// `self` moved `t` (0 to 1) of the way to `other`.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let l = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(l(self.0, other.0), l(self.1, other.1), l(self.2, other.2))
    }

    /// WCAG contrast ratio between two colours, 1 to 21.
    pub fn contrast(self, other: Rgb) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn luminance(self) -> f32 {
        let c = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(self.0) + 0.7152 * c(self.1) + 0.0722 * c(self.2)
    }
}

/// Something in the editor that has a colour of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Background,
    Text,
    LineNumbers,
    Selection,
    Cursor,
    ActiveLine,
    Syntax(Scope),
}

const EDITOR_SLOTS: [Slot; 6] = [
    Slot::Background,
    Slot::Text,
    Slot::LineNumbers,
    Slot::Selection,
    Slot::Cursor,
    Slot::ActiveLine,
];

impl Slot {
    /// The key used in `settings.json`: `editor.background`, `syntax.keyword`…
    pub fn key(self) -> String {
        match self {
            Slot::Background => "editor.background".into(),
            Slot::Text => "editor.text".into(),
            Slot::LineNumbers => "editor.line_numbers".into(),
            Slot::Selection => "editor.selection".into(),
            Slot::Cursor => "editor.cursor".into(),
            Slot::ActiveLine => "editor.active_line".into(),
            Slot::Syntax(s) => format!("syntax.{}", s.key()),
        }
    }

    pub fn from_key(key: &str) -> Option<Slot> {
        EDITOR_SLOTS
            .into_iter()
            .chain(Scope::ALL.into_iter().map(Slot::Syntax))
            .find(|s| s.key() == key)
    }

    /// Every slot, the editor's own first and then each kind of code.
    pub fn all() -> impl Iterator<Item = Slot> {
        EDITOR_SLOTS
            .into_iter()
            .chain(Scope::ALL.into_iter().map(Slot::Syntax))
    }

    fn index(self) -> usize {
        match self {
            Slot::Background => 0,
            Slot::Text => 1,
            Slot::LineNumbers => 2,
            Slot::Selection => 3,
            Slot::Cursor => 4,
            Slot::ActiveLine => 5,
            Slot::Syntax(s) => {
                EDITOR_SLOTS.len() + Scope::ALL.iter().position(|x| *x == s).unwrap_or(0)
            }
        }
    }
}

const SLOT_COUNT: usize = EDITOR_SLOTS.len() + Scope::ALL.len();

/// Whether `scope` is drawn in italics.
pub fn italic(scope: Scope) -> bool {
    matches!(scope, Scope::Comment | Scope::DocComment | Scope::Emphasis)
}

/// Every colour of the editor, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    slots: [Rgb; SLOT_COUNT],
}

impl Colors {
    /// All slots set to one colour, to be filled in.
    pub fn filled(color: Rgb) -> Colors {
        Colors {
            slots: [color; SLOT_COUNT],
        }
    }

    pub fn get(&self, slot: Slot) -> Rgb {
        self.slots
            .get(slot.index())
            .copied()
            .unwrap_or(Rgb(0, 0, 0))
    }

    pub fn set(&mut self, slot: Slot, color: Rgb) {
        if let Some(s) = self.slots.get_mut(slot.index()) {
            *s = color;
        }
    }

    pub fn scope(&self, scope: Scope) -> Rgb {
        self.get(Slot::Syntax(scope))
    }

    /// Replace the colours a settings file names. Keys that are not slots and values that are
    /// not colours are skipped, so a hand-edited file cannot break the editor.
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) {
        for (key, value) in overrides {
            if let (Some(slot), Some(color)) = (Slot::from_key(key), Rgb::parse(value)) {
                self.set(slot, color);
            }
        }
    }
}

/// A built-in theme, described by a dozen colours; the rest are derived from them.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// Whether the editor is dark, which decides the window's appearance when it is picked.
    pub dark: bool,
    background: u32,
    text: u32,
    comment: u32,
    selection: u32,
    cursor: u32,
    active_line: u32,
    keyword: u32,
    string: u32,
    number: u32,
    function: u32,
    ty: u32,
    accent: u32,
}

impl Preset {
    pub fn colors(&self) -> Colors {
        let c = Rgb::from_u24;
        let (bg, fg, comment) = (c(self.background), c(self.text), c(self.comment));
        let (keyword, string, number) = (c(self.keyword), c(self.string), c(self.number));
        let (function, ty, accent) = (c(self.function), c(self.ty), c(self.accent));
        let quiet = fg.mix(comment, 0.45);
        let mut out = Colors::filled(fg);
        out.set(Slot::Background, bg);
        out.set(Slot::Text, fg);
        out.set(Slot::LineNumbers, bg.mix(fg, 0.62));
        out.set(Slot::Selection, c(self.selection));
        out.set(Slot::Cursor, c(self.cursor));
        out.set(Slot::ActiveLine, c(self.active_line));
        for scope in Scope::ALL {
            let color = match scope {
                Scope::Keyword | Scope::Control | Scope::Tag | Scope::Heading => keyword,
                Scope::Operator | Scope::Punctuation | Scope::Bracket => quiet,
                Scope::String | Scope::Literal => string,
                Scope::Escape => string.mix(number, 0.5),
                Scope::Number | Scope::Boolean | Scope::Constant => number,
                Scope::Comment | Scope::DocComment => comment,
                Scope::Function | Scope::Method | Scope::Link => function,
                Scope::Macro | Scope::Label | Scope::Attribute => accent,
                Scope::Type | Scope::BuiltinType | Scope::Namespace => ty,
                Scope::Property => fg.mix(function, 0.35),
                Scope::Variable | Scope::Parameter | Scope::Emphasis => fg,
            };
            out.set(Slot::Syntax(scope), color);
        }
        out
    }
}

/// The id of the theme that follows the window's own colours.
pub const NODALIA: &str = "nodalia";

macro_rules! preset {
    ($id:expr, $name:expr, $dark:expr, $bg:expr, $fg:expr, $comment:expr, $sel:expr, $cur:expr,
     $line:expr, $kw:expr, $str:expr, $num:expr, $func:expr, $ty:expr, $acc:expr) => {
        Preset {
            id: $id,
            name: $name,
            dark: $dark,
            background: $bg,
            text: $fg,
            comment: $comment,
            selection: $sel,
            cursor: $cur,
            active_line: $line,
            keyword: $kw,
            string: $str,
            number: $num,
            function: $func,
            ty: $ty,
            accent: $acc,
        }
    };
}

/// The themes Código ships besides the Nodalia one.
pub const PRESETS: &[Preset] = &[
    preset!(
        "dracula", "Dracula", true, 0x282a36, 0xf8f8f2, 0x8a95c7, 0x44475a, 0xf8f8f0, 0x2f3242,
        0xff79c6, 0xf1fa8c, 0xbd93f9, 0x50fa7b, 0x8be9fd, 0xffb86c
    ),
    preset!(
        "one-dark", "One Dark", true, 0x282c34, 0xabb2bf, 0x7f848e, 0x3e4451, 0x528bff, 0x2c313c,
        0xc678dd, 0x98c379, 0xd19a66, 0x61afef, 0xe5c07b, 0x56b6c2
    ),
    preset!(
        "monokai", "Monokai", true, 0x272822, 0xf8f8f2, 0x8f8b73, 0x49483e, 0xf8f8f0, 0x3e3d32,
        0xf92672, 0xe6db74, 0xae81ff, 0xa6e22e, 0x66d9ef, 0xfd971f
    ),
    preset!(
        "solarized-dark",
        "Solarized Dark",
        true,
        0x002b36,
        0x93a1a1,
        0x7a8f93,
        0x073642,
        0x93a1a1,
        0x073642,
        0x859900,
        0x2aa198,
        0xd33682,
        0x268bd2,
        0xb58900,
        0xcb4b16
    ),
    preset!(
        "solarized-light",
        "Solarized Light",
        false,
        0xfdf6e3,
        0x4b6169,
        0x7d8b8b,
        0xeee8d5,
        0x586e75,
        0xf3ecd6,
        0x6f7e00,
        0x1f7f77,
        0xc02b74,
        0x1d78b5,
        0x9a7300,
        0xb5400f
    ),
    preset!(
        "github-light",
        "GitHub Light",
        false,
        0xffffff,
        0x24292f,
        0x6e7781,
        0xb6d7ff,
        0x0969da,
        0xf6f8fa,
        0xcf222e,
        0x0a3069,
        0x0550ae,
        0x8250df,
        0x953800,
        0x116329
    ),
    preset!(
        "github-dark",
        "GitHub Dark",
        true,
        0x0d1117,
        0xc9d1d9,
        0x8b949e,
        0x264f78,
        0x58a6ff,
        0x161b22,
        0xff7b72,
        0xa5d6ff,
        0x79c0ff,
        0xd2a8ff,
        0xffa657,
        0x7ee787
    ),
    preset!(
        "nord", "Nord", true, 0x2e3440, 0xd8dee9, 0x7b88a1, 0x434c5e, 0xd8dee9, 0x3b4252, 0x81a1c1,
        0xa3be8c, 0xb48ead, 0x88c0d0, 0x8fbcbb, 0xebcb8b
    ),
    preset!(
        "gruvbox-dark",
        "Gruvbox Dark",
        true,
        0x282828,
        0xebdbb2,
        0x928374,
        0x504945,
        0xebdbb2,
        0x32302f,
        0xfb4934,
        0xb8bb26,
        0xd3869b,
        0xfabd2f,
        0x83a598,
        0xfe8019
    ),
    preset!(
        "gruvbox-light",
        "Gruvbox Light",
        false,
        0xfbf1c7,
        0x3c3836,
        0x7c6f64,
        0xd5c4a1,
        0x3c3836,
        0xf2e5bc,
        0x9d0006,
        0x79740e,
        0x8f3f71,
        0x076678,
        0x8a5a00,
        0xaf3a03
    ),
    preset!(
        "tokyo-night",
        "Tokyo Night",
        true,
        0x1a1b26,
        0xc0caf5,
        0x7982a9,
        0x283457,
        0xc0caf5,
        0x1f2335,
        0xbb9af7,
        0x9ece6a,
        0xff9e64,
        0x7aa2f7,
        0x2ac3de,
        0xe0af68
    ),
    preset!(
        "catppuccin-mocha",
        "Catppuccin Mocha",
        true,
        0x1e1e2e,
        0xcdd6f4,
        0x8087a2,
        0x45475a,
        0xf5e0dc,
        0x262637,
        0xcba6f7,
        0xa6e3a1,
        0xfab387,
        0x89b4fa,
        0xf9e2af,
        0x94e2d5
    ),
    preset!(
        "catppuccin-latte",
        "Catppuccin Latte",
        false,
        0xeff1f5,
        0x4c4f69,
        0x7c7f93,
        0xccd0da,
        0xdc8a78,
        0xe6e9ef,
        0x8839ef,
        0x2f7d1f,
        0xc2410c,
        0x1e66f5,
        0x96640a,
        0x0f766e
    ),
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Rgb::parse("#0b9a80"), Some(Rgb(11, 154, 128)));
        assert_eq!(Rgb::parse("0B9A80"), Some(Rgb(11, 154, 128)));
        assert_eq!(Rgb::parse("#fa0"), Some(Rgb(255, 170, 0)));
        assert_eq!(Rgb(11, 154, 128).hex(), "#0b9a80");
        for bad in [
            "",
            "#",
            "#12",
            "#12345",
            "#gggggg",
            "#ñññ",
            "rgb(1,2,3)",
            "#1234567",
        ] {
            assert_eq!(Rgb::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn mixing_and_contrast_behave() {
        let (black, white) = (Rgb(0, 0, 0), Rgb(255, 255, 255));
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, f32::NAN), black);
        assert!((black.contrast(white) - 21.0).abs() < 0.01);
        assert!((white.contrast(white) - 1.0).abs() < 0.01);
    }

    #[test]
    fn every_slot_key_finds_its_slot() {
        let mut seen = std::collections::HashSet::new();
        for s in Slot::all() {
            assert!(seen.insert(s.key()), "{s:?} repeats a key");
            assert_eq!(Slot::from_key(&s.key()), Some(s));
            assert!(s.index() < SLOT_COUNT);
        }
        assert_eq!(seen.len(), SLOT_COUNT);
        assert_eq!(Slot::from_key("syntax.nope"), None);
    }

    #[test]
    fn presets_have_unique_ids_and_never_shadow_the_nodalia_theme() {
        let mut ids = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(ids.insert(p.id), "{} repeats", p.id);
            assert_ne!(p.id, NODALIA);
            assert_eq!(preset(p.id).map(|x| x.id), Some(p.id));
        }
        assert!(preset("missing").is_none());
    }

    #[test]
    fn every_preset_is_readable() {
        let mut problems = Vec::new();
        for p in PRESETS {
            let c = p.colors();
            let bg = c.get(Slot::Background);
            // Light themes are light and dark ones dark, so the window can follow them.
            if (bg.contrast(Rgb(0, 0, 0)) > bg.contrast(Rgb(255, 255, 255))) == p.dark {
                problems.push(format!("{}: dark flag is wrong", p.id));
            }
            let mut check = |what: String, ratio: f32, min: f32| {
                if ratio < min {
                    problems.push(format!("{}: {what} contrast {ratio:.2} < {min}", p.id));
                }
            };
            check("text".into(), c.get(Slot::Text).contrast(bg), 4.5);
            for scope in Scope::ALL {
                check(format!("{scope:?}"), c.scope(scope).contrast(bg), 3.0);
            }
            check(
                "line numbers".into(),
                c.get(Slot::LineNumbers).contrast(bg),
                2.5,
            );
            // The selection and the current line must not swallow the text.
            for slot in [Slot::Selection, Slot::ActiveLine] {
                check(
                    format!("text on {slot:?}"),
                    c.get(Slot::Text).contrast(c.get(slot)),
                    4.5,
                );
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn overrides_replace_only_what_they_name_and_ignore_nonsense() {
        let mut c = preset("nord").expect("nord").colors();
        let before = c.clone();
        let mut o = BTreeMap::new();
        o.insert("editor.background".to_owned(), "#101010".to_owned());
        o.insert("syntax.keyword".to_owned(), "#ff0000".to_owned());
        o.insert("syntax.keyword2".to_owned(), "#00ff00".to_owned());
        o.insert("syntax.string".to_owned(), "not a colour".to_owned());
        c.apply_overrides(&o);
        assert_eq!(c.get(Slot::Background), Rgb(16, 16, 16));
        assert_eq!(c.scope(Scope::Keyword), Rgb(255, 0, 0));
        assert_eq!(c.scope(Scope::String), before.scope(Scope::String));
        assert_eq!(c.get(Slot::Text), before.get(Slot::Text));
    }
}
