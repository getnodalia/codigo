// SPDX-License-Identifier: MIT OR Apache-2.0
//! Merge conflicts: a window that lists the blocks git left in a file and keeps the side the
//! person picks, one block at a time. The file is read and written on worker threads.

use std::path::PathBuf;

use codigo_git::{Conflict, Side};
use egui::{FontId, RichText, ScrollArea};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};
use crate::vcs_ui::{self, GitMsg};

/// Files bigger than this are not opened here; a merge in one is for the editor.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// Lines of each side shown before it is cut.
const SHOWN_LINES: usize = 14;

pub struct ConflictView {
    /// Relative to the repository root.
    pub path: String,
    /// `None` while the file is being read.
    pub text: Option<Result<String, String>>,
}

/// Open the window for a conflicted file.
pub fn open(app: &mut App, path: String) {
    let Some(root) = app.vcs.root.clone() else {
        return;
    };
    app.save_all();
    app.overlay = Overlay::Conflicts(ConflictView {
        path: path.clone(),
        text: None,
    });
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-conflict".into())
        .spawn(move || {
            let file = root.join(&path);
            let result = match std::fs::metadata(&file) {
                Ok(m) if m.len() > MAX_FILE => {
                    Err("this file is too big to resolve here; open it in the editor".to_owned())
                }
                Ok(_) => std::fs::read_to_string(&file)
                    .map_err(|e| format!("could not read {path}: {e}")),
                Err(e) => Err(format!("could not read {path}: {e}")),
            };
            let _ = tx.send(crate::app::Msg::Vcs(GitMsg::Conflicted { path, result }));
            ctx.request_repaint();
        });
}

/// The file arrived.
pub fn loaded(app: &mut App, path: String, result: Result<String, String>) {
    if let Overlay::Conflicts(v) = &mut app.overlay {
        if v.path == path {
            v.text = Some(result);
        }
    }
}

/// Write the new text of the file; the panel refreshes when it is done.
fn write(app: &mut App, path: &str, text: String) {
    let Some(root) = app.vcs.root.clone() else {
        return;
    };
    let file: PathBuf = root.join(path);
    vcs_ui::run_job(app, "Resolved the conflict", move || {
        std::fs::write(&file, text)
            .map(|()| String::new())
            .map_err(|e| format!("could not write {}: {e}", file.display()))
    });
}

pub fn overlay(ctx: &egui::Context, app: &mut App, look: &Look, mut view: ConflictView) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut chosen: Option<(usize, Side)> = None;
    let mut finish = false;
    let idle = app.vcs.busy.is_none();
    let dismissed = modal(ctx, look, "conflicts", 860.0, true, |ui| {
        closed = heading(ui, look, &view.path, "compare");
        match &view.text {
            None => {
                ui.label(RichText::new(tr("Reading the file…")).color(color(p.text_secondary)));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(color(p.status_danger_fg)));
            }
            Some(Ok(text)) => {
                let all = codigo_git::conflicts(text);
                if all.is_empty() {
                    ui.label(
                        RichText::new(tr("No conflicts are left in this file."))
                            .color(color(p.status_success_fg)),
                    );
                    ui.add_space(8.0);
                    if cr::primary_button(ui, look, tr("Mark as resolved"), idle).clicked() {
                        finish = true;
                    }
                } else {
                    ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 260.0).max(200.0))
                        .show(ui, |ui| {
                            for (i, c) in all.iter().enumerate() {
                                block(ui, look, text, i, all.len(), c, idle, &mut chosen);
                                ui.add_space(10.0);
                            }
                        });
                }
            }
        }
    });
    if let (Some((index, side)), Some(Ok(text))) = (chosen, view.text.clone()) {
        if let Some(next) = codigo_git::resolve(&text, index, side) {
            view.text = Some(Ok(next.clone()));
            let path = view.path.clone();
            write(app, &path, next);
        }
    }
    if finish {
        let path = vec![view.path.clone()];
        vcs_ui::run_repo(app, "Marked as resolved", move |repo, _| {
            repo.stage(&path).map(|()| String::new())
        });
        return (Overlay::None, true);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::Conflicts(view), false)
}

#[allow(clippy::too_many_arguments)]
fn block(
    ui: &mut egui::Ui,
    look: &Look,
    text: &str,
    index: usize,
    total: usize,
    c: &Conflict,
    idle: bool,
    chosen: &mut Option<(usize, Side)>,
) {
    let p = look.palette;
    ui.label(
        RichText::new(trf(
            "Conflict {} of {}",
            &[&(index + 1).to_string(), &total.to_string()],
        ))
        .strong()
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    let sides = [
        (
            Side::Ours,
            &c.ours_label,
            p.status_success_fg,
            p.status_success_bg,
        ),
        (
            Side::Theirs,
            &c.theirs_label,
            p.status_info_fg,
            p.status_info_bg,
        ),
    ];
    for (side, label, fg, bg) in sides {
        let name = match side {
            Side::Ours => tr("Current"),
            _ => tr("Incoming"),
        };
        ui.label(
            RichText::new(format!("{name} · {label}"))
                .size(font_size::SMALL)
                .color(color(fg)),
        );
        let body = codigo_git::side_text(text, c, side);
        let mut shown: Vec<&str> = body.lines().take(SHOWN_LINES).collect();
        let cut = body.lines().count() > SHOWN_LINES;
        if cut {
            shown.push("…");
        }
        egui::Frame::new()
            .fill(color(bg).gamma_multiply(0.5))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if shown.is_empty() {
                    ui.label(
                        RichText::new(tr("(nothing)"))
                            .size(font_size::SMALL)
                            .color(color(p.text_tertiary)),
                    );
                }
                for l in shown {
                    ui.add(
                        egui::Label::new(
                            RichText::new(l)
                                .font(FontId::monospace(font_size::SMALL))
                                .color(color(p.text_primary)),
                        )
                        .extend(),
                    );
                }
            });
    }
    ui.horizontal(|ui| {
        if cr::secondary_button(ui, look, tr("Keep current"), idle).clicked() {
            *chosen = Some((index, Side::Ours));
        }
        if cr::secondary_button(ui, look, tr("Keep incoming"), idle).clicked() {
            *chosen = Some((index, Side::Theirs));
        }
        if cr::secondary_button(ui, look, tr("Keep both"), idle).clicked() {
            *chosen = Some((index, Side::Both));
        }
    });
}
