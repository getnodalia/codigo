// SPDX-License-Identifier: MIT OR Apache-2.0
//! Small pieces drawn with the NDL palette: icons, icon buttons, toggles and the status chips.

use egui::{vec2, Color32, CornerRadius, FontId, Response, Sense, Stroke, Ui};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;
use nodalia_theme::icons;

/// Paint an NDL icon in a square of `size` points.
pub fn icon(ui: &mut Ui, name: &str, size: f32, ink: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if let Some(i) = icons::get(name) {
        icons::paint(ui.painter(), rect, i, icons::Style::bold(ink));
    }
    response
}

/// A round, borderless icon button with an accessible label.
pub fn icon_button(ui: &mut Ui, look: &Look, name: &str, label: &str, active: bool) -> Response {
    let size = 28.0;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    paint_icon_button(ui, look, rect, &response, name, active);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

/// A smaller icon button for dense rows (tabs, the find bar).
pub fn small_icon_button(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    label: &str,
    active: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    paint_icon_button(ui, look, rect, &response, name, active);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

fn paint_icon_button(
    ui: &Ui,
    look: &Look,
    rect: egui::Rect,
    response: &Response,
    name: &str,
    active: bool,
) {
    let p = &look.palette;
    let fill = if active {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(10),
            Stroke::new(2.0, color(p.border_focus)),
            egui::StrokeKind::Outside,
        );
    }
    let ink = if active {
        color(p.accent_text)
    } else {
        color(p.text_secondary)
    };
    let inset = (rect.width() * 0.22).round();
    paint_named(ui.painter(), rect.shrink(inset), name, ink);
}

/// Paint an icon by name: the Nodalia set, or one of the few drawn in `glyphs`.
pub fn paint_named(painter: &egui::Painter, rect: egui::Rect, name: &str, ink: Color32) {
    use crate::glyphs::{self, Glyph};
    if let Some(brand) = crate::brand::Brand::from_icon_name(name) {
        crate::brand::paint(painter, rect, brand, ink);
        return;
    }
    let glyph = match name {
        "bug" => Some(Glyph::Bug),
        "step-over" => Some(Glyph::StepOver),
        "step-out" => Some(Glyph::StepOut),
        _ => None,
    };
    if let Some(g) = glyph {
        glyphs::paint(painter, rect, g, ink);
    } else if let Some(i) = icons::get(name) {
        icons::paint(painter, rect, i, icons::Style::bold(ink));
    }
}

/// A round button of the top bar's action cluster: tinted when it can be used, grey when not.
/// Returns true when it was clicked while enabled.
pub fn tool_button(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    label: &str,
    hint: &str,
    enabled: bool,
    tint: Color32,
) -> bool {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(14), color(p.control_bg_hover));
    }
    let ink = if enabled {
        tint
    } else {
        color(p.text_tertiary)
    };
    paint_named(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), vec2(15.0, 15.0)),
        name,
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response.on_hover_text(hint).clicked() && enabled
}

/// A small text toggle such as "Aa" or ".*": pressed it wears the accent.
pub fn toggle_chip(ui: &mut Ui, look: &Look, text: &str, label: &str, on: bool) -> Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
    let fill = if on {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    let ink = if on {
        color(p.accent_text)
    } else {
        color(p.text_secondary)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::monospace(12.0),
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, label));
    response.on_hover_text(label)
}

/// A labelled tab chip: an icon and a word, as wide as they need. `toggle_chip` is a fixed box
/// for two characters, and a longer label drew over its neighbours.
pub fn tab_chip(ui: &mut Ui, look: &Look, icon_name: &str, label: &str, on: bool) -> Response {
    let p = &look.palette;
    let ink = if on {
        color(p.accent_text)
    } else {
        color(p.text_secondary)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.0), ink);
    let (pad, icon_size, gap) = (8.0, 14.0, 5.0);
    let size = vec2(
        pad * 2.0 + icon_size + gap + galley.size().x,
        galley.size().y.max(icon_size) + 10.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if on {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad, rect.center().y - icon_size / 2.0),
        vec2(icon_size, icon_size),
    );
    paint_named(ui.painter(), icon_rect, icon_name, ink);
    let text_pos = egui::pos2(
        icon_rect.right() + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, ink);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label));
    response
}

/// Lay out `add` from the right edge, on the current row. `with_layout` would claim the whole
/// height left in a horizontal row and push the row's height to it.
pub fn trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let row = if ui.layout().is_horizontal() {
        ui.min_rect().height()
    } else {
        0.0
    };
    let height = row.max(ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodalia_theme::{Accent, Mode};

    /// The panels' tab rows once drew five labels in boxes made for two characters, on top of one
    /// another. Each chip must take the room of its own label, and a narrow sidebar must wrap.
    #[test]
    fn tab_chips_do_not_overlap_and_wrap_inside_a_narrow_sidebar() {
        let look = Look::new(Mode::Dark.palette_with(Accent::Codigo));
        let ctx = egui::Context::default();
        let labels = [
            "Pull requests",
            "Issues",
            "Mine",
            "To review",
            "Repositories",
        ];
        let mut rects = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_max_width(330.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for label in labels {
                    rects.push(tab_chip(ui, &look, "branch", label, false).rect);
                }
            });
        });
        out.textures_delta.clear();
        assert_eq!(rects.len(), labels.len());
        for (i, a) in rects.iter().enumerate() {
            assert!(a.width() > 60.0, "chip {i} is only {} wide", a.width());
            assert!(a.right() <= 340.0, "chip {i} runs out of the sidebar");
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
        }
        assert!(
            rects.iter().any(|r| r.top() > rects[0].top()),
            "five labelled chips must wrap in 330 points"
        );
    }
}
