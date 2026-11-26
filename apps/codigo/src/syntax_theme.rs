// SPDX-License-Identifier: MIT OR Apache-2.0
//! Syntax colours, derived from the Nodalia palette so a theme change (light, dark, high
//! contrast) recolours code too and every colour keeps the palette's contrast.

use codigo_syntax::Scope;
use egui::Color32;
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

/// How one kind of text is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub color: Color32,
    pub italic: bool,
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// The ink of `scope`, or `None` for plain text.
pub fn ink(look: &Look, scope: Scope) -> Option<Ink> {
    let p = &look.palette;
    let plain = color(p.text_primary);
    let accent = color(p.accent_text);
    let blue = color(p.status_info_fg);
    let amber = color(p.status_warning_fg);
    let red = color(p.status_danger_fg);
    let green = color(p.status_success_fg);
    let quiet = color(p.text_tertiary);
    let soft = color(p.text_secondary);
    let (c, italic) = match scope {
        Scope::Keyword | Scope::Control => (accent, false),
        Scope::Operator => (soft, false),
        Scope::Punctuation | Scope::Bracket => (soft, false),
        Scope::String => (amber, false),
        Scope::Escape => (mix(amber, red, 0.5), false),
        Scope::Number | Scope::Constant => (mix(red, amber, 0.35), false),
        Scope::Boolean => (mix(red, amber, 0.35), false),
        Scope::Comment | Scope::DocComment => (quiet, true),
        Scope::Function | Scope::Method => (blue, false),
        Scope::Macro => (mix(blue, accent, 0.5), false),
        Scope::Type | Scope::BuiltinType => (mix(green, blue, 0.4), false),
        Scope::Variable => (plain, false),
        Scope::Parameter => (mix(plain, amber, 0.25), false),
        Scope::Property => (mix(plain, blue, 0.35), false),
        Scope::Attribute => (mix(blue, quiet, 0.4), false),
        Scope::Namespace => (mix(plain, green, 0.4), false),
        Scope::Label => (mix(accent, amber, 0.5), false),
        Scope::Tag => (accent, false),
        Scope::Heading => (accent, false),
        Scope::Emphasis => (plain, true),
        Scope::Link => (color(p.text_link), false),
        Scope::Literal => (amber, false),
    };
    Some(Ink { color: c, italic })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodalia_theme::{contrast, Accent, Mode};

    #[test]
    fn every_scope_is_readable_on_the_editor_surface_in_every_mode() {
        let scopes = [
            Scope::Keyword,
            Scope::Control,
            Scope::Operator,
            Scope::Punctuation,
            Scope::Bracket,
            Scope::String,
            Scope::Escape,
            Scope::Number,
            Scope::Constant,
            Scope::Boolean,
            Scope::Comment,
            Scope::DocComment,
            Scope::Function,
            Scope::Method,
            Scope::Macro,
            Scope::Type,
            Scope::BuiltinType,
            Scope::Variable,
            Scope::Parameter,
            Scope::Property,
            Scope::Attribute,
            Scope::Namespace,
            Scope::Label,
            Scope::Tag,
            Scope::Heading,
            Scope::Emphasis,
            Scope::Link,
            Scope::Literal,
        ];
        for mode in [Mode::Light, Mode::Dark, Mode::LightHc, Mode::DarkHc] {
            let palette = mode.palette_with(Accent::Codigo);
            let look = Look::new(palette);
            for scope in scopes {
                let Some(i) = ink(&look, scope) else { continue };
                let ratio = contrast(
                    nodalia_theme::Rgba::new(i.color.r(), i.color.g(), i.color.b(), 255),
                    palette.surface_panel,
                );
                assert!(ratio >= 3.0, "{scope:?} in {mode:?}: contrast {ratio:.2}");
            }
        }
    }
}
