// SPDX-License-Identifier: MIT OR Apache-2.0
//! Syntax colours, derived from the Nodalia palette so a theme change (light, dark, high
//! contrast) recolours code too and every colour keeps the palette's contrast.

use codigo_syntax::Scope;
use nodalia_theme::creator::Look;

use codigo_engine::settings::Settings;
use codigo_engine::theme::{self, Colors, Rgb, Slot};
use nodalia_theme::{Palette, Rgba};

fn rgb(c: Rgba) -> Rgb {
    Rgb(c.r, c.g, c.b)
}

fn rgba(c: Rgb) -> Rgba {
    Rgba::new(c.0, c.1, c.2, 255)
}

/// The colours the Nodalia theme gives the editor: every one comes from the window's palette, so
/// light, dark and high contrast recolour code too and keep the palette's contrast.
pub fn nodalia_colors(p: &Palette) -> Colors {
    let plain = rgb(p.text_primary);
    let accent = rgb(p.accent_text);
    let blue = rgb(p.status_info_fg);
    let amber = rgb(p.status_warning_fg);
    let red = rgb(p.status_danger_fg);
    let green = rgb(p.status_success_fg);
    let quiet = rgb(p.text_tertiary);
    let soft = rgb(p.text_secondary);
    let mut out = Colors::filled(plain);
    out.set(Slot::Background, rgb(p.surface_panel));
    out.set(Slot::Text, plain);
    out.set(Slot::LineNumbers, quiet);
    out.set(Slot::Selection, rgb(p.selection_bg));
    out.set(Slot::Cursor, rgb(p.accent_default));
    out.set(
        Slot::ActiveLine,
        rgb(p.surface_panel).mix(rgb(p.control_bg), 0.55),
    );
    for scope in Scope::ALL {
        let c = match scope {
            Scope::Keyword | Scope::Control | Scope::Tag | Scope::Heading => accent,
            Scope::Operator | Scope::Punctuation | Scope::Bracket => soft,
            Scope::String | Scope::Literal => amber,
            Scope::Escape => amber.mix(red, 0.5),
            Scope::Number | Scope::Constant | Scope::Boolean => red.mix(amber, 0.35),
            Scope::Comment | Scope::DocComment => quiet,
            Scope::Function | Scope::Method => blue,
            Scope::Macro => blue.mix(accent, 0.5),
            Scope::Type | Scope::BuiltinType => green.mix(blue, 0.4),
            Scope::Variable | Scope::Emphasis => plain,
            Scope::Parameter => plain.mix(amber, 0.25),
            Scope::Property => plain.mix(blue, 0.35),
            Scope::Attribute => blue.mix(quiet, 0.4),
            Scope::Namespace => plain.mix(green, 0.4),
            Scope::Label => accent.mix(amber, 0.5),
            Scope::Link => rgb(p.text_link),
        };
        out.set(Slot::Syntax(scope), c);
    }
    out
}

/// The colours to draw the editor with: the chosen theme (or the window's own colours), then the
/// person's changes on top.
pub fn editor_colors(settings: &Settings, palette: &Palette) -> Colors {
    let mut colors = theme::preset(&settings.color_theme)
        .map_or_else(|| nodalia_colors(palette), |p| p.colors());
    colors.apply_overrides(&settings.color_overrides);
    colors
}

/// The window's look with the editor's colours put in the slots the editor reads, so the text
/// area, its gutter and its minimap follow the theme while the rest of the window does not.
pub fn editor_look(look: &Look, colors: &Colors) -> Look {
    let mut p = look.palette;
    let (bg, fg) = (colors.get(Slot::Background), colors.get(Slot::Text));
    p.surface_panel = rgba(bg);
    p.surface_window = rgba(bg.mix(fg, 0.05));
    p.text_primary = rgba(fg);
    p.text_secondary = rgba(bg.mix(fg, 0.7));
    p.text_tertiary = rgba(colors.get(Slot::LineNumbers));
    p.selection_bg = rgba(colors.get(Slot::Selection));
    p.accent_default = rgba(colors.get(Slot::Cursor));
    p.border_subtle = rgba(bg.mix(fg, 0.12));
    Look::new(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodalia_theme::{Accent, Mode};

    #[test]
    fn every_scope_is_readable_on_the_editor_surface_in_every_mode() {
        for mode in [Mode::Light, Mode::Dark, Mode::LightHc, Mode::DarkHc] {
            let palette = mode.palette_with(Accent::Codigo);
            let colors = nodalia_colors(&palette);
            let bg = colors.get(Slot::Background);
            for scope in Scope::ALL {
                let ratio = colors.scope(scope).contrast(bg);
                assert!(ratio >= 3.0, "{scope:?} in {mode:?}: contrast {ratio:.2}");
            }
        }
    }

    #[test]
    fn the_editor_look_carries_the_theme_and_keeps_the_rest_of_the_palette() {
        let palette = Mode::Dark.palette_with(Accent::Codigo);
        let settings = Settings {
            color_theme: "dracula".into(),
            ..Settings::default()
        };
        let colors = editor_colors(&settings, &palette);
        let look = editor_look(&Look::new(palette), &colors);
        assert_eq!(rgb(look.palette.surface_panel), Rgb(0x28, 0x2a, 0x36));
        assert_eq!(rgb(look.palette.text_primary), Rgb(0xf8, 0xf8, 0xf2));
        assert_eq!(look.palette.status_danger_fg, palette.status_danger_fg);
    }

    #[test]
    fn the_nodalia_theme_follows_the_palette_and_overrides_still_apply() {
        let light = Mode::Light.palette_with(Accent::Codigo);
        let dark = Mode::Dark.palette_with(Accent::Codigo);
        let mut settings = Settings::default();
        assert_ne!(
            editor_colors(&settings, &light).get(Slot::Background),
            editor_colors(&settings, &dark).get(Slot::Background)
        );
        settings
            .color_overrides
            .insert("syntax.keyword".into(), "#123456".into());
        assert_eq!(
            editor_colors(&settings, &dark).scope(Scope::Keyword),
            Rgb(0x12, 0x34, 0x56)
        );
    }
}
