// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Problems panel in the sidebar: the errors and warnings the language servers reported, for
//! every file they know, errors first. A click opens the file at that line.

use std::path::PathBuf;

use codigo_engine::Severity;
use egui::{pos2, vec2, CornerRadius, FontId, RichText, ScrollArea, Sense, Ui};
use nodalia_theme::creator::Look;
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};

const ROW_H: f32 = 52.0;

/// A problem to open: the file and its 0-based line.
pub type Open = (PathBuf, usize);

pub fn panel(ui: &mut Ui, app: &App, look: &Look) -> Option<Open> {
    let p = &look.palette;
    let mut open = None;
    if !app.lsp.enabled() {
        ui.label(
            RichText::new(tr(
                "Language servers are turned off. Turn them on in Preferences to see errors here.",
            ))
            .color(color(p.text_secondary))
            .size(font_size::SMALL),
        );
        return None;
    }
    let totals = app.lsp.totals();
    ui.label(
        RichText::new(trf(
            "{} errors, {} warnings",
            &[&totals.errors.to_string(), &totals.warnings.to_string()],
        ))
        .strong()
        .color(color(if totals.errors > 0 {
            p.status_danger_fg
        } else if totals.warnings > 0 {
            p.status_warning_fg
        } else {
            p.status_success_fg
        })),
    );
    ui.add_space(6.0);
    let problems = app.lsp.problems();
    if problems.is_empty() {
        ui.label(
            RichText::new(tr(
                "No problems in the files a language server has looked at.",
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        return None;
    }
    let root = app.project.as_ref().map(|p| p.root.clone());
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("problems")
        .show_rows(ui, ROW_H, problems.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let Some(item) = problems.get(i) else {
                    continue;
                };
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(7),
                        color(p.control_bg_hover),
                    );
                }
                let painter = ui.painter_at(rect);
                let ink = match item.severity {
                    Severity::Error => p.status_danger_fg,
                    _ => p.status_warning_fg,
                };
                painter.circle_filled(pos2(rect.left() + 9.0, rect.top() + 13.0), 4.0, color(ink));
                let first_line = item.message.lines().next().unwrap_or_default();
                let headline = painter.layout(
                    first_line.to_owned(),
                    FontId::proportional(font_size::SMALL + 1.0),
                    color(p.text_primary),
                    rect.width() - 26.0,
                );
                let below = rect.top() + 5.0 + headline.size().y.min(32.0) + 9.0;
                painter.galley(
                    pos2(rect.left() + 20.0, rect.top() + 5.0),
                    headline,
                    color(p.text_primary),
                );
                let shown = root
                    .as_ref()
                    .and_then(|r| item.path.strip_prefix(r).ok())
                    .unwrap_or(&item.path);
                let source = item
                    .source
                    .as_deref()
                    .map(|s| format!("{s} · "))
                    .unwrap_or_default();
                painter.text(
                    pos2(rect.left() + 20.0, below),
                    egui::Align2::LEFT_CENTER,
                    format!("{source}{}:{}", shown.to_string_lossy(), item.line + 1),
                    FontId::proportional(font_size::SMALL),
                    color(p.text_secondary),
                );
                if response.clicked() {
                    open = Some((item.path.clone(), item.line));
                }
                response.on_hover_text(&item.message);
            }
        });
    open
}
