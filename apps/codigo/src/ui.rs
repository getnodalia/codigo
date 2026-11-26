// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window: top bar, sidebar, editor island and status bar. Everything here only draws and
//! collects what the person did into [`Act`]s; the app carries them out after the frame, so no
//! widget holds a borrow of the state it changes.

use std::path::PathBuf;

use codigo_engine::Cmd;
use codigo_text::LineEnding;
use codigo_workspace::{GitInfo, Status};
use egui::{
    pos2, vec2, Align, Color32, CornerRadius, CursorIcon, FontId, Key, Layout, Margin, Rect,
    RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder,
};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::for_egui::color;
use nodalia_theme::{font_size, Mode};

use crate::app::{App, Panel};
use crate::i18n::{tr, trf};
use crate::overlays::{Confirm, Overlay};
use crate::widgets::{icon, icon_button, small_icon_button, toggle_chip, trailing};
use crate::{chrome, editor, overlays};

const STATUS_H: f32 = 24.0;
const TAB_H: f32 = 34.0;
const ROW_H: f32 = 24.0;
const MIN_SIDEBAR: f32 = 200.0;
const MAX_SIDEBAR: f32 = 520.0;

/// What the person asked for during a frame.
enum Act {
    Run(Cmd),
    Updates,
    Tab(usize),
    CloseTab(usize),
    Toggle(PathBuf),
    OpenFile(PathBuf),
    OpenResult {
        path: PathBuf,
        line: usize,
        start: usize,
        end: usize,
    },
    OpenProject(PathBuf),
    CloseProject,
    Panel(Panel),
    ProjectSearch,
    FindNext,
    FindPrevious,
    ReplaceOne,
    ReplaceAll,
    CloseFind,
    RefreshExplorer,
    ResolveConflict(usize),
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    let mut acts: Vec<Act> = Vec::new();
    let gap = layout::GAP;
    let bar = full.with_max_y(full.top() + layout::APP_BAR);
    top_bar(ui, bar, app, &look, mode, ndl, &mut acts);

    let status = Rect::from_min_max(
        pos2(full.left() + gap, full.bottom() - STATUS_H),
        pos2(full.right() - gap, full.bottom()),
    );
    let body = Rect::from_min_max(
        pos2(full.left() + gap, bar.bottom()),
        pos2(full.right() - gap, status.top()),
    );
    let side_width = app.sidebar_width.clamp(MIN_SIDEBAR, MAX_SIDEBAR);
    let show_sidebar = app.sidebar_visible && body.width() > side_width + 360.0;
    let center = if show_sidebar {
        let side = Rect::from_min_max(body.min, pos2(body.left() + side_width, body.bottom()));
        island(ui, &look, side, "sidebar", |ui| {
            sidebar(ui, app, &look, &mut acts)
        });
        let handle = Rect::from_min_max(
            pos2(side.right(), body.top()),
            pos2(side.right() + gap, body.bottom()),
        );
        let response = ui
            .interact(handle, ui.scope_id().with("sidebar-resize"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        if response.dragged() {
            app.sidebar_width =
                (side_width + response.drag_delta().x).clamp(MIN_SIDEBAR, MAX_SIDEBAR);
            app.session_dirty = true;
        }
        Rect::from_min_max(pos2(side.right() + gap, body.top()), body.max)
    } else {
        body
    };
    island(ui, &look, center, "editor", |ui| {
        workspace(ui, app, &look, &mut acts)
    });
    status_bar(ui, status, app, &look, &mut acts);

    overlays::show(&ctx, app, &look);
    toasts(ui, app, &look);
    if ndl {
        chrome::resize_edges(ui, full);
    }
    for act in acts {
        apply(&ctx, app, act);
    }
}

fn apply(ctx: &egui::Context, app: &mut App, act: Act) {
    match act {
        Act::Run(cmd) => app.run(ctx, cmd),
        Act::Updates => app.updates.start(true),
        Act::Tab(i) => {
            if i < app.tabs.len() {
                app.active = i;
                app.focus_editor = true;
                app.session_dirty = true;
            }
        }
        Act::CloseTab(i) => app.close_tab(i),
        Act::Toggle(dir) => {
            if let Some(p) = app.project.as_mut() {
                p.explorer.toggle(&dir);
            }
        }
        Act::OpenFile(path) => {
            app.open_path(&path, None, true);
        }
        Act::OpenResult {
            path,
            line,
            start,
            end,
        } => {
            if app.open_path(&path, None, true) {
                app.select_range(line, start, end);
            }
        }
        Act::OpenProject(path) => {
            if path.is_dir() {
                app.open_project(path, true);
            } else {
                app.toast(tr("That folder no longer exists."));
            }
        }
        Act::CloseProject => app.close_project(),
        Act::Panel(panel) => {
            app.panel = panel;
            if panel == Panel::Search {
                app.psearch.focus = true;
            }
        }
        Act::ProjectSearch => app.run_project_search(),
        Act::FindNext => app.find_step(true),
        Act::FindPrevious => app.find_step(false),
        Act::ReplaceOne => app.replace_current(),
        Act::ReplaceAll => app.replace_all(),
        Act::CloseFind => app.close_find(),
        Act::RefreshExplorer => {
            if let Some(p) = app.project.as_mut() {
                p.explorer.refresh();
            }
            app.refresh_git();
            app.list_files();
        }
        Act::ResolveConflict(i) => app.overlay = Overlay::Confirm(Confirm::Conflict(i)),
    }
}

pub fn island(ui: &mut Ui, look: &Look, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        cr::island(look).show(ui, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            add(ui);
        });
    });
}

// --- Top bar -----------------------------------------------------------------------------------

fn top_bar(
    ui: &mut Ui,
    bar: Rect,
    app: &mut App,
    look: &Look,
    mode: Mode,
    ndl: bool,
    acts: &mut Vec<Act>,
) {
    let p = &look.palette;
    if chrome::drags_window(ndl) {
        chrome::drag_area(ui, bar);
    }
    let controls_left = ndl && chrome::side() == nodalia_theme::titlebar::Side::Left;
    let right_reserved = if ndl && !controls_left {
        chrome::CONTROLS_WIDTH
    } else {
        layout::GAP
    };
    let left_reserved = if controls_left {
        chrome::CONTROLS_WIDTH + layout::GAP
    } else {
        layout::GAP + chrome::leading_inset()
    };
    let inner = Rect::from_min_max(
        pos2(bar.left() + left_reserved, bar.top()),
        pos2(bar.right() - right_reserved, bar.bottom()),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let menu = icon_button(ui, look, "menu", tr("Main menu"), false);
            egui::Popup::menu(&menu).show(|ui| {
                ui.set_min_width(220.0);
                for (label, cmd) in [
                    (tr("Preferences"), Cmd::Preferences),
                    (tr("Keyboard shortcuts"), Cmd::KeyboardShortcuts),
                    (tr("About Código"), Cmd::About),
                ] {
                    if ui.button(label).clicked() {
                        acts.push(Act::Run(cmd));
                    }
                }
                if ui.button(tr("Check for updates…")).clicked() {
                    acts.push(Act::Updates);
                }
                ui.separator();
                for (label, cmd) in [
                    (tr("Open folder…"), Cmd::OpenFolder),
                    (tr("Open file…"), Cmd::OpenFile),
                    (tr("New file"), Cmd::NewFile),
                ] {
                    if ui.button(label).clicked() {
                        acts.push(Act::Run(cmd));
                    }
                }
                ui.separator();
                if ui.button(tr("Quit")).clicked() {
                    acts.push(Act::Run(Cmd::Quit));
                }
            });
            if icon_button(
                ui,
                look,
                "sidebar",
                tr("Show or hide the sidebar"),
                app.sidebar_visible,
            )
            .clicked()
            {
                acts.push(Act::Run(Cmd::ToggleSidebar));
            }
            ui.add_space(4.0);
            logo(ui, 22.0);
            ui.label(
                RichText::new(crate::app::NAME)
                    .strong()
                    .size(font_size::LARGE)
                    .color(color(p.text_primary)),
            );

            // The palette, as a wide field in the middle of the bar.
            let rest = ui.available_width();
            let width = (rest - 300.0).clamp(160.0, 460.0);
            ui.add_space(((rest - width) / 2.0 - 120.0).max(8.0));
            let hint = match &app.project {
                Some(p) => p
                    .root
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                None => tr("Search files and commands").to_owned(),
            };
            if palette_button(ui, look, &hint, width).clicked() {
                acts.push(Act::Run(if app.project.is_some() {
                    Cmd::QuickOpen
                } else {
                    Cmd::CommandPalette
                }));
            }
        },
    );
    if ndl {
        chrome::controls(ui, bar, p, mode);
    }
}

fn palette_button(ui: &mut Ui, look: &Look, text: &str, width: f32) -> egui::Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let fill = if response.hovered() {
        color(p.control_bg_hover)
    } else {
        color(p.control_field_bg)
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(layout::FIELD_RADIUS), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(layout::FIELD_RADIUS),
        Stroke::new(1.0, color(p.border_default)),
        egui::StrokeKind::Inside,
    );
    if let Some(i) = nodalia_theme::icons::get("search") {
        let glyph =
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0));
        nodalia_theme::icons::paint(
            ui.painter(),
            glyph,
            i,
            nodalia_theme::icons::Style::bold(color(p.text_tertiary)),
        );
    }
    ui.painter().text(
        pos2(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(font_size::BODY),
        color(p.text_secondary),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    response
}

/// The app icon, drawn from the embedded PNG.
pub fn logo(ui: &mut Ui, size: f32) {
    let texture = crate::icon::texture(ui.ctx());
    ui.add(
        egui::Image::new(&texture)
            .fit_to_exact_size(vec2(size, size))
            .corner_radius(CornerRadius::same((size * 0.22) as u8)),
    );
}

// --- Sidebar -----------------------------------------------------------------------------------

fn sidebar(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
    ui.horizontal(|ui| {
        for (panel, label) in [
            (Panel::Explorer, tr("Files")),
            (Panel::Search, tr("Search")),
        ] {
            if cr::tab(ui, look, label, app.panel == panel).clicked() {
                acts.push(Act::Panel(panel));
            }
        }
        trailing(ui, |ui| {
            if app.project.is_some()
                && small_icon_button(ui, look, "refresh", tr("Refresh"), false).clicked()
            {
                acts.push(Act::RefreshExplorer);
            }
        });
    });
    ui.add_space(4.0);
    match app.panel {
        Panel::Explorer => explorer(ui, app, look, acts),
        Panel::Search => search_panel(ui, app, look, acts),
    }
}

fn git_ink(look: &Look, status: Status) -> Color32 {
    let p = &look.palette;
    color(match status {
        Status::Modified | Status::Renamed => p.status_warning_fg,
        Status::Added | Status::Untracked => p.status_success_fg,
        Status::Deleted | Status::Conflicted => p.status_danger_fg,
    })
}

fn no_project(ui: &mut Ui, app: &App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    ui.add_space(12.0);
    ui.label(
        RichText::new(tr("No folder is open."))
            .color(color(p.text_secondary))
            .size(font_size::BODY),
    );
    ui.add_space(8.0);
    if cr::primary_button(ui, look, tr("Open folder…"), true).clicked() {
        acts.push(Act::Run(Cmd::OpenFolder));
    }
    if !app.settings.recent_projects.is_empty() {
        ui.add_space(16.0);
        ui.label(
            RichText::new(tr("Recent folders"))
                .strong()
                .color(color(p.text_primary)),
        );
        for path in app.settings.recent_projects.iter().take(8) {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            if cr::home_row(ui, look, "folder", &name, false)
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                acts.push(Act::OpenProject(path.clone()));
            }
        }
    }
}

fn explorer(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    let Some(project) = app.project.as_ref() else {
        no_project(ui, app, look, acts);
        return;
    };
    let name = project
        .root
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name.to_uppercase())
                .strong()
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        trailing(ui, |ui| {
            if small_icon_button(ui, look, "close", tr("Close folder"), false).clicked() {
                acts.push(Act::CloseProject);
            }
        });
    });
    let rows = project.explorer.rows();
    let active_path = app
        .active_doc()
        .and_then(|d| d.path().map(std::path::Path::to_path_buf));
    let selected = project.explorer.selected.clone();
    let git: Option<&GitInfo> = project.git.as_ref();
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("explorer")
        .show_rows(ui, ROW_H, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in rows.get(range).unwrap_or_default() {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let is_active = active_path.as_ref() == Some(&row.entry.path)
                    || selected.as_ref() == Some(&row.entry.path);
                if is_active {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(7), color(p.accent_subtle));
                } else if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(7),
                        color(p.control_bg_hover),
                    );
                }
                let x = rect.left() + 6.0 + row.depth as f32 * 14.0;
                let status = git.and_then(|g| g.status_of(&row.entry.path));
                let mut ink = if row.entry.ignored {
                    color(p.text_tertiary)
                } else if is_active {
                    color(p.accent_text)
                } else {
                    color(p.text_primary)
                };
                if let Some(s) = status {
                    ink = git_ink(look, s);
                }
                let painter = ui.painter();
                if row.entry.is_dir {
                    let chevron = if row.open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    };
                    if let Some(i) = nodalia_theme::icons::get(chevron) {
                        nodalia_theme::icons::paint(
                            painter,
                            Rect::from_center_size(
                                pos2(x + 6.0, rect.center().y),
                                vec2(12.0, 12.0),
                            ),
                            i,
                            nodalia_theme::icons::Style::bold(color(p.text_tertiary)),
                        );
                    }
                }
                let glyph = if row.entry.is_dir {
                    "folder"
                } else {
                    "file-text"
                };
                if let Some(i) = nodalia_theme::icons::get(glyph) {
                    nodalia_theme::icons::paint(
                        painter,
                        Rect::from_center_size(pos2(x + 24.0, rect.center().y), vec2(14.0, 14.0)),
                        i,
                        nodalia_theme::icons::Style::bold(if row.entry.is_dir {
                            color(p.accent_default)
                        } else {
                            color(p.text_secondary)
                        }),
                    );
                }
                painter.text(
                    pos2(x + 38.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &row.entry.name,
                    FontId::proportional(font_size::BODY),
                    ink,
                );
                if response.clicked() {
                    if row.entry.is_dir {
                        acts.push(Act::Toggle(row.entry.path.clone()));
                    } else {
                        acts.push(Act::OpenFile(row.entry.path.clone()));
                    }
                }
            }
        });
}

fn search_panel(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else {
        no_project(ui, app, look, acts);
        return;
    };
    let mut changed = false;
    let focus = std::mem::take(&mut app.psearch.focus);
    cr::field_frame(look)
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.psearch.query.pattern)
                        .hint_text(tr("Search in the folder"))
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width() - 90.0)
                        .align(egui::Align2::LEFT_CENTER),
                );
                if focus {
                    response.request_focus();
                }
                if response.changed() {
                    changed = true;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    changed = true;
                }
                let q = &mut app.psearch.query;
                if toggle_chip(ui, look, "Aa", tr("Match case"), q.case_sensitive).clicked() {
                    q.case_sensitive = !q.case_sensitive;
                    changed = true;
                }
                if toggle_chip(ui, look, "ab", tr("Whole word"), q.whole_word).clicked() {
                    q.whole_word = !q.whole_word;
                    changed = true;
                }
                if toggle_chip(ui, look, ".*", tr("Regular expression"), q.regex).clicked() {
                    q.regex = !q.regex;
                    changed = true;
                }
            });
        });
    if changed {
        acts.push(Act::ProjectSearch);
    }
    ui.add_space(6.0);
    match &app.psearch.result {
        None => {
            if app.psearch.running {
                ui.label(RichText::new(tr("Searching…")).color(color(p.text_secondary)));
            }
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        Some(Ok(result)) => {
            let summary = if result.hits.is_empty() {
                tr("No results.").to_owned()
            } else {
                trf(
                    "{} results",
                    &[&format!(
                        "{}{}",
                        result.hits.len(),
                        if result.truncated { "+" } else { "" }
                    )],
                )
            };
            ui.label(
                RichText::new(summary)
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
            ui.add_space(2.0);
            // One row per hit, with the file's name above the first of its hits.
            let hits = &result.hits;
            ScrollArea::vertical()
                .auto_shrink(false)
                .id_salt("search-results")
                .show_rows(ui, ROW_H, hits.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for i in range {
                        let Some(hit) = hits.get(i) else { continue };
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                        if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(7),
                                color(p.control_bg_hover),
                            );
                        }
                        let file = hit.path.to_string_lossy();
                        let name = file.rsplit('/').next().unwrap_or(&file);
                        let preview = hit.preview.trim_start();
                        let painter = ui.painter();
                        let label = painter.layout_no_wrap(
                            format!("{name}:{}", hit.line + 1),
                            FontId::proportional(font_size::SMALL),
                            color(p.accent_text),
                        );
                        let label_w = label.size().x;
                        painter.galley(
                            pos2(rect.left() + 6.0, rect.center().y - label.size().y / 2.0),
                            label,
                            color(p.accent_text),
                        );
                        let text = painter.layout_no_wrap(
                            preview.to_owned(),
                            FontId::monospace(font_size::SMALL),
                            color(p.text_secondary),
                        );
                        let clip = painter.clip_rect().intersect(rect);
                        painter.with_clip_rect(clip).galley(
                            pos2(
                                rect.left() + 14.0 + label_w,
                                rect.center().y - text.size().y / 2.0,
                            ),
                            text,
                            color(p.text_secondary),
                        );
                        if response.clicked() {
                            let trimmed = hit.preview.len() - preview.len();
                            acts.push(Act::OpenResult {
                                path: root.join(&hit.path),
                                line: hit.line,
                                start: hit.start,
                                end: hit.end,
                            });
                            let _ = trimmed;
                        }
                        response.on_hover_text(file.into_owned());
                    }
                });
        }
    }
}

// --- Editor island -----------------------------------------------------------------------------

fn workspace(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    if app.tabs.is_empty() {
        welcome(ui, app, look, acts);
        return;
    }
    tab_strip(ui, app, look, acts);
    let conflict = app.active_doc().is_some_and(|d| d.conflict);
    if conflict {
        banner(
            ui,
            look,
            tr("This file changed on disk while you had unsaved edits."),
            |ui| {
                if cr::secondary_button(ui, look, tr("Resolve…"), true).clicked() {
                    acts.push(Act::ResolveConflict(app.active));
                }
            },
        );
    }
    if app.find.open {
        find_bar(ui, app, look, acts);
    }
    let focus = std::mem::take(&mut app.focus_editor)
        && matches!(app.overlay, Overlay::None)
        && !app.find.focus;
    let opts = app.editor_options();
    let now = ui.ctx().input(|i| i.time);
    let rect = ui.available_rect_before_wrap();
    let active = app.active;
    let output = {
        let App { tabs, find, .. } = app;
        let matcher = if find.open {
            find.matcher.as_ref()
        } else {
            None
        };
        let Some(tab) = tabs.get_mut(active) else {
            return;
        };
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            editor::show(
                ui,
                &mut tab.doc,
                &mut tab.view,
                &opts,
                look,
                matcher,
                focus,
                now,
            )
        })
        .inner
    };
    let ctx = ui.ctx().clone();
    app.editor_output(&ctx, output);
}

fn banner(ui: &mut Ui, look: &Look, text: &str, add: impl FnOnce(&mut Ui)) {
    let p = &look.palette;
    egui::Frame::NONE
        .fill(color(p.status_warning_bg))
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(color(p.status_warning_fg)));
                trailing(ui, add);
            });
        });
}

fn tab_strip(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_H), Sense::hover());
    ui.painter().line_segment(
        [strip.left_bottom(), strip.right_bottom()],
        Stroke::new(1.0, color(p.border_default)),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(strip)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.add_space(6.0);
            ScrollArea::horizontal()
                .id_salt("tabs")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .max_width(strip.width() - 40.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (i, tab) in app.tabs.iter().enumerate() {
                            let name = tab
                                .doc
                                .file_name()
                                .unwrap_or_else(|| tr("Untitled").to_owned());
                            let dirty = tab.doc.is_dirty();
                            let selected = i == app.active;
                            file_tab(ui, look, &name, dirty, selected, i, acts, selected);
                        }
                    });
                });
            if small_icon_button(ui, look, "plus", tr("New file"), false).clicked() {
                acts.push(Act::Run(Cmd::NewFile));
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn file_tab(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    dirty: bool,
    selected: bool,
    index: usize,
    acts: &mut Vec<Act>,
    scroll_to: bool,
) {
    let p = &look.palette;
    let font = FontId::proportional(font_size::BODY);
    let ink = if selected {
        color(p.text_primary)
    } else {
        color(p.text_secondary)
    };
    let galley = ui.painter().layout_no_wrap(name.to_owned(), font, ink);
    let width = galley.size().x + 50.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    if selected && scroll_to {
        ui.scroll_to_rect(rect, None);
    }
    let fill = if selected {
        color(p.control_bg_hover)
    } else if response.hovered() {
        color(p.control_bg_hover).gamma_multiply(0.5)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    let close =
        Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(18.0, 18.0));
    let over_close = response
        .hover_pos()
        .is_some_and(|pos| close.expand(2.0).contains(pos));
    if dirty && !(response.hovered() && over_close) {
        ui.painter()
            .circle_filled(close.center(), 4.0, color(p.accent_default));
    } else if selected || response.hovered() {
        if over_close {
            ui.painter()
                .rect_filled(close, CornerRadius::same(6), color(p.control_bg_hover));
        }
        if let Some(i) = nodalia_theme::icons::get("close") {
            nodalia_theme::icons::paint(
                ui.painter(),
                close.shrink(4.0),
                i,
                nodalia_theme::icons::Style::bold(color(p.text_secondary)),
            );
        }
    }
    if response.middle_clicked() || (response.clicked() && over_close) {
        acts.push(Act::CloseTab(index));
    } else if response.clicked() {
        acts.push(Act::Tab(index));
    }
}

fn find_bar(ui: &mut Ui, app: &mut App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.horizontal(|ui| {
                cr::field_frame(look)
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        let width = (ui.available_width() - 250.0).clamp(140.0, 360.0);
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut app.find.query.pattern)
                                .hint_text(tr("Find"))
                                .frame(egui::Frame::NONE)
                                .desired_width(width)
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        if std::mem::take(&mut app.find.focus) {
                            response.request_focus();
                            // Select the pattern so typing replaces it.
                            if let Some(mut state) =
                                egui::TextEdit::load_state(ui.ctx(), response.id)
                            {
                                let len = app.find.query.pattern.chars().count();
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(len),
                                    )));
                                state.store(ui.ctx(), response.id);
                            }
                        }
                        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            let back = ui.input(|i| i.modifiers.shift);
                            acts.push(if back {
                                Act::FindPrevious
                            } else {
                                Act::FindNext
                            });
                            response.request_focus();
                        }
                        let q = &mut app.find.query;
                        if toggle_chip(ui, look, "Aa", tr("Match case"), q.case_sensitive).clicked()
                        {
                            q.case_sensitive = !q.case_sensitive;
                        }
                        if toggle_chip(ui, look, "ab", tr("Whole word"), q.whole_word).clicked() {
                            q.whole_word = !q.whole_word;
                        }
                        if toggle_chip(ui, look, ".*", tr("Regular expression"), q.regex).clicked()
                        {
                            q.regex = !q.regex;
                        }
                    });
                let info = if let Some(e) = &app.find.error {
                    (e.clone(), p.status_danger_fg)
                } else if app.find.query.pattern.is_empty() {
                    (String::new(), p.text_secondary)
                } else if app.find.count == 0 {
                    (tr("No results").to_owned(), p.status_warning_fg)
                } else {
                    let n = app.find.count.to_string();
                    match app.find.current {
                        Some(c) => (
                            trf("{} of {}", &[&(c + 1).to_string(), &n]),
                            p.text_secondary,
                        ),
                        None => (trf("{} matches", &[&n]), p.text_secondary),
                    }
                };
                ui.label(
                    RichText::new(info.0)
                        .size(font_size::SMALL)
                        .color(color(info.1)),
                );
                if small_icon_button(ui, look, "arrow-up", tr("Previous match"), false).clicked() {
                    acts.push(Act::FindPrevious);
                }
                if small_icon_button(ui, look, "arrow-down", tr("Next match"), false).clicked() {
                    acts.push(Act::FindNext);
                }
                if small_icon_button(ui, look, "replace", tr("Show replace"), app.find.replace)
                    .clicked()
                {
                    app.find.replace = !app.find.replace;
                }
                trailing(ui, |ui| {
                    if small_icon_button(ui, look, "close", tr("Close"), false).clicked() {
                        acts.push(Act::CloseFind);
                    }
                });
            });
            if app.find.replace {
                ui.horizontal(|ui| {
                    cr::field_frame(look)
                        .inner_margin(Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            let width = (ui.available_width() - 250.0).clamp(140.0, 360.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut app.find.with)
                                    .hint_text(tr("Replace with"))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(width + 78.0)
                                    .align(egui::Align2::LEFT_CENTER),
                            );
                        });
                    if cr::secondary_button(ui, look, tr("Replace"), true).clicked() {
                        acts.push(Act::ReplaceOne);
                    }
                    if cr::secondary_button(ui, look, tr("Replace all"), true).clicked() {
                        acts.push(Act::ReplaceAll);
                    }
                });
            }
        });
}

fn welcome(ui: &mut Ui, app: &App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    let rect = ui.max_rect();
    let width = 420.0_f32.min(rect.width() - 40.0);
    let content = Rect::from_center_size(
        pos2(
            rect.center().x,
            rect.top() + (rect.height() * 0.42).max(220.0),
        ),
        vec2(width, 10.0),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(content.left(), rect.top() + 56.0),
                pos2(content.right(), rect.bottom() - 16.0),
            ))
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            logo(ui, 72.0);
            ui.add_space(10.0);
            ui.label(
                RichText::new(crate::app::NAME)
                    .strong()
                    .size(font_size::TITLE)
                    .color(color(p.text_primary)),
            );
            ui.label(
                RichText::new(tr("Code in any language, with AI beside you."))
                    .color(color(p.text_secondary)),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let total = 120.0 + 100.0 + 120.0 + 16.0;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                if cr::primary_button_min(ui, look, tr("Open folder…"), true, 120.0).clicked() {
                    acts.push(Act::Run(Cmd::OpenFolder));
                }
                if cr::secondary_button_min(ui, look, tr("Open file…"), true, 100.0).clicked() {
                    acts.push(Act::Run(Cmd::OpenFile));
                }
                if cr::secondary_button_min(ui, look, tr("New file"), true, 120.0).clicked() {
                    acts.push(Act::Run(Cmd::NewFile));
                }
            });
            if !app.settings.recent_projects.is_empty() {
                ui.add_space(22.0);
                ui.label(
                    RichText::new(tr("Recent folders"))
                        .strong()
                        .color(color(p.text_primary)),
                );
                ui.add_space(4.0);
                for path in app.settings.recent_projects.iter().take(6) {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    if cr::home_row(ui, look, "folder", &name, false)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        acts.push(Act::OpenProject(path.clone()));
                    }
                }
            }
            ui.add_space(20.0);
            let hint = |ui: &mut Ui, cmd: Cmd| {
                let keys = cmd
                    .shortcut()
                    .map(|s| shortcut_label(&s))
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!("{}   {keys}", tr(cmd.label())))
                        .size(font_size::SMALL)
                        .color(color(p.text_tertiary)),
                );
            };
            hint(ui, Cmd::CommandPalette);
            hint(ui, Cmd::QuickOpen);
        },
    );
}

pub fn shortcut_label(s: &codigo_engine::Shortcut) -> String {
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    if s.primary {
        parts.push(if mac { "⌘" } else { "Ctrl" }.to_owned());
    }
    if s.alt {
        parts.push(if mac { "⌥" } else { "Alt" }.to_owned());
    }
    if s.shift {
        parts.push(if mac { "⇧" } else { "Shift" }.to_owned());
    }
    parts.push(s.key.label().to_owned());
    parts.join(if mac { "" } else { "+" })
}

// --- Status bar --------------------------------------------------------------------------------

fn status_bar(ui: &mut Ui, rect: Rect, app: &App, look: &Look, acts: &mut Vec<Act>) {
    let p = &look.palette;
    let ink = color(p.text_secondary);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            ui.add_space(6.0);
            let small = |text: String| RichText::new(text).size(font_size::SMALL).color(ink);
            if let Some(git) = app.project.as_ref().and_then(|p| p.git.as_ref()) {
                icon(ui, "branch", 13.0, ink);
                let branch = git
                    .branch
                    .clone()
                    .unwrap_or_else(|| tr("detached").to_owned());
                let mut text = branch;
                if git.ahead > 0 {
                    text.push_str(&format!(" ↑{}", git.ahead));
                }
                if git.behind > 0 {
                    text.push_str(&format!(" ↓{}", git.behind));
                }
                ui.label(small(text));
            }
            let Some(doc) = app.active_doc() else {
                return;
            };
            if let Some(path) = doc.path() {
                let counts = app.lsp.counts(path);
                if counts.errors + counts.warnings > 0 {
                    let tip = trf(
                        "{} errors, {} warnings",
                        &[&counts.errors.to_string(), &counts.warnings.to_string()],
                    );
                    ui.label(
                        RichText::new(format!("✕ {}", counts.errors))
                            .size(font_size::SMALL)
                            .color(color(p.status_danger_fg)),
                    )
                    .on_hover_text(tip.clone());
                    ui.label(
                        RichText::new(format!("▲ {}", counts.warnings))
                            .size(font_size::SMALL)
                            .color(color(p.status_warning_fg)),
                    )
                    .on_hover_text(tip);
                }
            }
            if let Some(status) = app.lsp.status_for(doc.language().id) {
                use codigo_engine::ServerState;
                let name = status.name.to_owned();
                match status.state {
                    ServerState::Starting => {
                        ui.label(small(trf("{} is starting…", &[&name])));
                    }
                    ServerState::Ready(None) => {
                        ui.label(small(name));
                    }
                    ServerState::Ready(Some(progress)) => {
                        ui.label(small(format!("{name} · {progress}")));
                    }
                    ServerState::Missing(install) => {
                        ui.label(small(trf("{} is not installed", &[&name])))
                            .on_hover_text(trf("Install it with: {}", &[install]));
                    }
                    ServerState::Failed(why) => {
                        ui.label(small(trf("{} stopped", &[&name])))
                            .on_hover_text(why);
                    }
                }
            }
            trailing(ui, |ui| {
                ui.add_space(6.0);
                let language = ui
                    .add(
                        egui::Label::new(small(doc.language().name.to_owned()))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(tr("Change the language of this file"));
                if language.clicked() {
                    acts.push(Act::Run(Cmd::ChangeLanguage));
                }
                ui.label(small("UTF-8".to_owned()));
                ui.label(small(
                    match doc.line_ending() {
                        LineEnding::Lf => "LF",
                        LineEnding::CrLf => "CRLF",
                    }
                    .to_owned(),
                ));
                ui.label(small(match doc.buffer().indent {
                    codigo_text::Indent::Tabs => tr("Tabs").to_owned(),
                    codigo_text::Indent::Spaces(n) => trf("Spaces: {}", &[&n.to_string()]),
                }));
                let b = doc.buffer();
                let (line, col) = b.line_col(b.primary().head);
                let position = ui
                    .add(
                        egui::Label::new(small(trf(
                            "Line {}, column {}",
                            &[&(line + 1).to_string(), &(col + 1).to_string()],
                        )))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(tr("Go to line…"));
                if position.clicked() {
                    acts.push(Act::Run(Cmd::GoToLine));
                }
                let n = b.selections().len();
                if n > 1 {
                    ui.label(small(trf("{} cursors", &[&n.to_string()])));
                }
            });
        },
    );
}

// --- Toasts ------------------------------------------------------------------------------------

fn toasts(ui: &mut Ui, app: &App, look: &Look) {
    let p = &look.palette;
    let full = ui.max_rect();
    let mut y = full.bottom() - STATUS_H - 12.0;
    for toast in app.toasts.iter().rev() {
        let galley = ui.painter().layout(
            toast.text.clone(),
            FontId::proportional(font_size::BODY),
            color(p.text_primary),
            360.0,
        );
        let size = galley.size() + vec2(28.0, 18.0);
        let rect = Rect::from_min_size(pos2(full.right() - size.x - 16.0, y - size.y), size);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("codigo-toast"),
        ));
        painter.rect_filled(rect, CornerRadius::same(12), color(p.surface_panel));
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(1.0, color(p.border_default)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + vec2(14.0, 9.0), galley, color(p.text_primary));
        y = rect.top() - 8.0;
    }
}
