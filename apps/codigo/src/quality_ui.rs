// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Quality panel in the sidebar: the gate, the counts by severity and the findings of the
//! last analysis, worst first. A click on a finding opens the file at that place.

use std::path::PathBuf;

use codigo_quality::{Finding, Gate, Kind, Severity};
use egui::{pos2, vec2, CornerRadius, FontId, RichText, ScrollArea, Sense, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};

const ROW_H: f32 = 52.0;

/// What the person did in the panel.
#[derive(Default)]
pub struct Outcome {
    pub analyze: bool,
    /// A finding to open: file, 0-based line, and the byte columns to select.
    pub open: Option<(PathBuf, usize, usize, usize)>,
}

pub fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Blocker => tr("Blocker"),
        Severity::Critical => tr("Critical"),
        Severity::Major => tr("Major"),
        Severity::Minor => tr("Minor"),
        Severity::Info => tr("Info"),
    }
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Bug => tr("Bug"),
        Kind::Vulnerability => tr("Vulnerability"),
        Kind::Hotspot => tr("Security hotspot"),
        Kind::Secret => tr("Secret"),
        Kind::Smell => tr("Code smell"),
    }
}

/// A finding's sentence in the current language.
pub fn title(finding: &Finding) -> String {
    match &finding.detail {
        Some(detail) => trf(finding.rule.title, &[detail]),
        None => tr(finding.rule.title).to_owned(),
    }
}

fn ink(look: &Look, severity: Severity) -> egui::Color32 {
    let p = &look.palette;
    color(match severity {
        Severity::Blocker | Severity::Critical => p.status_danger_fg,
        Severity::Major => p.status_warning_fg,
        Severity::Minor => p.status_info_fg,
        Severity::Info => p.text_tertiary,
    })
}

pub fn panel(ui: &mut Ui, app: &App, look: &Look) -> Outcome {
    let p = &look.palette;
    let mut out = Outcome::default();
    let state = &app.quality;
    ui.horizontal(|ui| {
        let label = if state.running {
            tr("Analyzing…")
        } else {
            tr("Analyze")
        };
        if cr::primary_button(ui, look, label, !state.running).clicked() {
            out.analyze = true;
        }
    });
    ui.add_space(6.0);
    let Some(report) = state.report.as_ref() else {
        if !state.running {
            ui.label(
                RichText::new(tr(
                    "Look for hard-coded secrets, unsafe code, complex functions and copied code.",
                ))
                .color(color(p.text_secondary))
                .size(font_size::SMALL),
            );
        }
        return out;
    };

    let (gate_text, gate_ink) = match report.gate() {
        Gate::Passed => (tr("Quality gate passed"), p.status_success_fg),
        Gate::Warning => (tr("Quality gate: needs attention"), p.status_warning_fg),
        Gate::Failed => (tr("Quality gate failed"), p.status_danger_fg),
    };
    ui.label(RichText::new(gate_text).strong().color(color(gate_ink)));
    ui.label(
        RichText::new(trf(
            "{} files, {} lines",
            &[&report.files.to_string(), &report.lines.to_string()],
        ))
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for severity in [
            Severity::Blocker,
            Severity::Critical,
            Severity::Major,
            Severity::Minor,
            Severity::Info,
        ] {
            let n = report.count(severity);
            let (rect, response) = ui.allocate_exact_size(vec2(44.0, 20.0), Sense::hover());
            let painter = ui.painter();
            painter.circle_filled(
                pos2(rect.left() + 6.0, rect.center().y),
                4.0,
                ink(look, severity),
            );
            painter.text(
                pos2(rect.left() + 15.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                n.to_string(),
                FontId::proportional(font_size::SMALL),
                color(if n == 0 {
                    p.text_tertiary
                } else {
                    p.text_primary
                }),
            );
            response.on_hover_text(severity_name(severity));
        }
    });
    ui.add_space(6.0);
    if report.findings.is_empty() {
        ui.label(RichText::new(tr("Nothing to report.")).color(color(p.text_secondary)));
        return out;
    }
    if report.truncated {
        ui.label(
            RichText::new(tr("Only the worst findings are shown."))
                .size(font_size::SMALL)
                .color(color(p.text_tertiary)),
        );
    }
    let findings = &report.findings;
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("quality-findings")
        .show_rows(ui, ROW_H, findings.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let Some(item) = findings.get(i) else {
                    continue;
                };
                let f = &item.finding;
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
                painter.circle_filled(
                    pos2(rect.left() + 9.0, rect.top() + 13.0),
                    4.0,
                    ink(look, f.severity),
                );
                let headline = painter.layout(
                    title(f),
                    FontId::proportional(font_size::SMALL + 1.0),
                    color(p.text_primary),
                    rect.width() - 26.0,
                );
                let below = rect.top() + 5.0 + headline.size().y + 9.0;
                painter.galley(
                    pos2(rect.left() + 20.0, rect.top() + 5.0),
                    headline,
                    color(p.text_primary),
                );
                let place = format!(
                    "{} · {}:{}",
                    kind_name(f.rule.kind),
                    item.path.to_string_lossy(),
                    f.line + 1
                );
                painter.text(
                    pos2(rect.left() + 20.0, below),
                    egui::Align2::LEFT_CENTER,
                    place,
                    FontId::proportional(font_size::SMALL),
                    color(p.text_secondary),
                );
                if response.clicked() {
                    out.open = Some((item.path.clone(), f.line, f.start, f.end));
                }
                response.on_hover_text(format!(
                    "{}\n{}",
                    severity_name(f.severity),
                    tr(f.rule.advice)
                ));
            }
        });
    out
}
