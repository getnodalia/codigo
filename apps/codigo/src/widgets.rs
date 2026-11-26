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
    if let Some(i) = icons::get(name) {
        let inset = (rect.width() * 0.22).round();
        icons::paint(ui.painter(), rect.shrink(inset), i, icons::Style::bold(ink));
    }
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
