// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything that opens over the editor: the command palette, Go to file, Go to line, the
//! language picker, Preferences, Keyboard shortcuts, About and the confirmations.

use codigo_engine::{Appearance, AutoSave, Category, Cmd};
use codigo_workspace::{rank, Ranked};
use egui::text::LayoutJob;
use egui::{
    vec2, Align, Align2, Color32, FontId, Id, Key, Layout, RichText, Sense, TextFormat, Ui,
};
use nodalia_theme::about::{self, Info, Link};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{self, tr, trf, Lang};

#[derive(Default)]
pub struct Picker {
    pub query: String,
    pub sel: usize,
    /// The query the cached ranking was made for, and the ranking.
    ranked_for: Option<String>,
    ranked: Vec<Ranked>,
    first: bool,
}

pub enum Confirm {
    /// Close these tabs; some have unsaved edits.
    Close(Vec<usize>),
    Quit,
    /// The file changed on disk while it had unsaved edits.
    Conflict(usize),
}

pub enum Overlay {
    None,
    Palette(Picker),
    QuickOpen(Picker),
    GoToLine(String),
    Language(Picker),
    Preferences,
    Shortcuts(String),
    About,
    Confirm(Confirm),
}

impl Overlay {
    pub fn palette() -> Overlay {
        Overlay::Palette(Picker {
            first: true,
            ..Picker::default()
        })
    }
    pub fn quick_open() -> Overlay {
        Overlay::QuickOpen(Picker {
            first: true,
            ..Picker::default()
        })
    }
    pub fn language() -> Overlay {
        Overlay::Language(Picker {
            first: true,
            ..Picker::default()
        })
    }
    pub fn go_to_line() -> Overlay {
        Overlay::GoToLine(String::new())
    }
}

/// A line of text with the characters at `positions` drawn in the accent colour.
fn highlighted(look: &Look, text: &str, positions: &[usize], size: f32) -> LayoutJob {
    let p = &look.palette;
    let mut job = LayoutJob::default();
    for (i, c) in text.chars().enumerate() {
        let hit = positions.contains(&i);
        job.append(
            &c.to_string(),
            0.0,
            TextFormat {
                font_id: FontId::proportional(size),
                color: if hit {
                    color(p.accent_text)
                } else {
                    color(p.text_primary)
                },
                underline: if hit {
                    egui::Stroke::new(1.0, color(p.accent_text))
                } else {
                    egui::Stroke::NONE
                },
                ..TextFormat::default()
            },
        );
    }
    job
}

struct Item {
    label: String,
    hint: String,
    positions: Vec<usize>,
}

/// A result list under a field. Returns the chosen index when one is picked.
fn picker_ui(
    ui: &mut Ui,
    look: &Look,
    picker: &mut Picker,
    hint: &str,
    items: &[Item],
    empty: &str,
) -> Option<usize> {
    let p = &look.palette;
    let mut choice = None;
    let response = cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut picker.query)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(font_size::LARGE)),
            )
        })
        .inner;
    if picker.first {
        response.request_focus();
        picker.first = false;
    }
    if response.changed() {
        picker.sel = 0;
    }
    let (down, up, enter) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
        )
    });
    if down && !items.is_empty() {
        picker.sel = (picker.sel + 1) % items.len();
    }
    if up && !items.is_empty() {
        picker.sel = (picker.sel + items.len() - 1) % items.len();
    }
    picker.sel = picker.sel.min(items.len().saturating_sub(1));
    if enter && !items.is_empty() {
        choice = Some(picker.sel);
    }
    ui.add_space(8.0);
    if items.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(empty).color(color(p.text_tertiary)));
        ui.add_space(6.0);
        return choice;
    }
    let row_h = 30.0;
    egui::ScrollArea::vertical()
        .id_salt("picker-list")
        .max_height(row_h * 11.0)
        .auto_shrink([false, true])
        .show_rows(ui, row_h, items.len(), |ui, range| {
            for i in range {
                let Some(item) = items.get(i) else { continue };
                let (rect, r) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let selected = i == picker.sel;
                if selected || r.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(layout::CONTROL_RADIUS),
                        if selected {
                            color(p.accent_subtle)
                        } else {
                            color(p.control_bg_hover)
                        },
                    );
                }
                let galley = ui.painter().layout_job(highlighted(
                    look,
                    &item.label,
                    &item.positions,
                    font_size::BODY,
                ));
                let hint_galley = ui.painter().layout_no_wrap(
                    item.hint.clone(),
                    FontId::proportional(font_size::SMALL),
                    color(p.text_tertiary),
                );
                let hint_w = hint_galley.size().x;
                ui.painter().galley(
                    egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    Color32::WHITE,
                );
                ui.painter().galley(
                    egui::pos2(
                        rect.right() - 10.0 - hint_w,
                        rect.center().y - hint_galley.size().y / 2.0,
                    ),
                    hint_galley,
                    Color32::WHITE,
                );
                if r.clicked() {
                    choice = Some(i);
                }
                if selected && (down || up) {
                    r.scroll_to_me(None);
                }
            }
        });
    choice
}

fn modal(
    ctx: &egui::Context,
    look: &Look,
    id: &'static str,
    width: f32,
    top: bool,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let p = &look.palette;
    let frame = cr::island(look)
        .inner_margin(16)
        .stroke(egui::Stroke::new(1.0, color(p.border_subtle)));
    let mut modal = egui::Modal::new(Id::unique(id)).frame(frame);
    if top {
        modal = modal.area(
            egui::Area::new(Id::unique(id))
                .kind(egui::UiKind::Modal)
                .anchor(Align2::CENTER_TOP, vec2(0.0, 84.0))
                .order(egui::Order::Foreground),
        );
    }
    let response = modal.show(ctx, |ui| {
        ui.set_width(width.min(ctx.content_rect().width() - 40.0));
        add(ui);
    });
    response.should_close()
}

pub fn show(ctx: &egui::Context, app: &mut App, look: &Look) {
    let overlay = std::mem::replace(&mut app.overlay, Overlay::None);
    let (next, close) = match overlay {
        Overlay::None => return,
        Overlay::Palette(picker) => palette(ctx, app, look, picker),
        Overlay::QuickOpen(picker) => quick_open(ctx, app, look, picker),
        Overlay::GoToLine(text) => go_to_line(ctx, app, look, text),
        Overlay::Language(picker) => language(ctx, app, look, picker),
        Overlay::Preferences => preferences(ctx, app, look),
        Overlay::Shortcuts(filter) => shortcuts(ctx, look, filter),
        Overlay::About => about_window(ctx, look),
        Overlay::Confirm(c) => confirm(ctx, app, look, c),
    };
    // A command run from inside an overlay may have opened another one: keep that.
    if matches!(app.overlay, Overlay::None) && !close {
        app.overlay = next;
    }
}

type Shown = (Overlay, bool);

fn palette(ctx: &egui::Context, app: &mut App, look: &Look, mut picker: Picker) -> Shown {
    let mac = cfg!(target_os = "macos");
    let all: Vec<Cmd> = Cmd::ALL.to_vec();
    let labels: Vec<&'static str> = all.iter().map(|c| tr(c.label())).collect();
    if picker.ranked_for.as_deref() != Some(picker.query.as_str()) {
        picker.ranked = rank(&picker.query, &labels, |l| l, 60);
        picker.ranked_for = Some(picker.query.clone());
    }
    let items: Vec<Item> = picker
        .ranked
        .iter()
        .filter_map(|r| {
            let cmd = all.get(r.index)?;
            Some(Item {
                label: (*labels.get(r.index)?).to_owned(),
                hint: cmd
                    .shortcut()
                    .map_or_else(|| tr(cmd.category().label()).to_owned(), |s| s.label(mac)),
                positions: r.positions.clone(),
            })
        })
        .collect();
    let mut chosen = None;
    let closed = modal(ctx, look, "palette", 600.0, true, |ui| {
        chosen = picker_ui(
            ui,
            look,
            &mut picker,
            tr("Type a command…"),
            &items,
            tr("No command matches."),
        );
    });
    if closed {
        return (Overlay::None, true);
    }
    if let Some(i) = chosen {
        if let Some(cmd) = picker.ranked.get(i).and_then(|r| all.get(r.index)).copied() {
            app.run(ctx, cmd);
            return (Overlay::None, true);
        }
    }
    (Overlay::Palette(picker), false)
}

fn quick_open(ctx: &egui::Context, app: &mut App, look: &Look, mut picker: Picker) -> Shown {
    let Some(project) = app.project.as_ref().map(|p| p.root.clone()) else {
        return (Overlay::None, true);
    };
    let files = app.files.as_ref().map(|(_, f)| f.clone());
    let stamp = format!("{}:{}", files.as_ref().map_or(0, |f| f.len()), picker.query);
    if picker.ranked_for.as_deref() != Some(stamp.as_str()) {
        picker.ranked = match &files {
            Some(f) => rank(&picker.query, f, |s| s.as_str(), 60),
            None => Vec::new(),
        };
        picker.ranked_for = Some(stamp);
    }
    let items: Vec<Item> = picker
        .ranked
        .iter()
        .filter_map(|r| {
            let path = files.as_ref()?.get(r.index)?;
            let (dir, name) = match path.rsplit_once('/') {
                Some((d, n)) => (d.to_owned(), n.to_owned()),
                None => (String::new(), path.clone()),
            };
            // Highlight positions are into the whole path; show the name and the folder.
            let offset = path.chars().count() - name.chars().count();
            let positions: Vec<usize> = r
                .positions
                .iter()
                .filter_map(|p| p.checked_sub(offset))
                .collect();
            Some(Item {
                label: name,
                hint: dir,
                positions,
            })
        })
        .collect();
    let mut chosen = None;
    let loading = files.is_none();
    let closed = modal(ctx, look, "quick-open", 620.0, true, |ui| {
        chosen = picker_ui(
            ui,
            look,
            &mut picker,
            tr("Go to file…"),
            &items,
            if loading {
                tr("Reading the project…")
            } else {
                tr("No file matches.")
            },
        );
    });
    if closed {
        return (Overlay::None, true);
    }
    if let Some(i) = chosen {
        if let Some(rel) = picker
            .ranked
            .get(i)
            .and_then(|r| files.as_ref()?.get(r.index))
        {
            app.open_path(&project.join(rel), None, true);
            return (Overlay::None, true);
        }
    }
    (Overlay::QuickOpen(picker), false)
}

fn go_to_line(ctx: &egui::Context, app: &mut App, look: &Look, mut text: String) -> Shown {
    let total = app.active_doc().map_or(0, |d| d.buffer().line_count());
    let mut go = false;
    let closed = modal(ctx, look, "go-to-line", 420.0, true, |ui| {
        let hint = trf("Line number (1–{})", &[&total.to_string()]);
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::LARGE)),
                )
            })
            .inner;
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        text.retain(|c| c.is_ascii_digit() || c == ':');
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
            go = true;
        }
    });
    if closed {
        return (Overlay::None, true);
    }
    if go {
        let line: usize = text
            .split(':')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        app.go_to_line(line.saturating_sub(1).min(total.saturating_sub(1)));
        app.focus_editor = true;
        return (Overlay::None, true);
    }
    (Overlay::GoToLine(text), false)
}

fn language(ctx: &egui::Context, app: &mut App, look: &Look, mut picker: Picker) -> Shown {
    let langs = codigo_syntax::languages();
    let names: Vec<&str> = langs.iter().map(|l| l.name).collect();
    if picker.ranked_for.as_deref() != Some(picker.query.as_str()) {
        picker.ranked = rank(&picker.query, &names, |n| n, 40);
        picker.ranked_for = Some(picker.query.clone());
    }
    let items: Vec<Item> = picker
        .ranked
        .iter()
        .filter_map(|r| {
            let l = langs.get(r.index)?;
            Some(Item {
                label: l.name.to_owned(),
                hint: if l.has_grammar() {
                    tr("Highlighting").to_owned()
                } else {
                    String::new()
                },
                positions: r.positions.clone(),
            })
        })
        .collect();
    let mut chosen = None;
    let closed = modal(ctx, look, "language", 480.0, true, |ui| {
        chosen = picker_ui(
            ui,
            look,
            &mut picker,
            tr("Select a language…"),
            &items,
            tr("No language matches."),
        );
    });
    if closed {
        return (Overlay::None, true);
    }
    if let Some(i) = chosen {
        if let Some(def) = picker.ranked.get(i).and_then(|r| langs.get(r.index)) {
            if let Some(tab) = app.tabs.get_mut(app.active) {
                tab.doc.set_language(def);
                tab.view.reveal = true;
            }
            app.focus_editor = true;
            return (Overlay::None, true);
        }
    }
    (Overlay::Language(picker), false)
}

fn heading(ui: &mut Ui, look: &Look, title: &str, id: &str) -> bool {
    let h = about::heading(ui, look, title, Id::unique(id).with("close"));
    h.closed
}

fn section(ui: &mut Ui, look: &Look, title: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title)
            .strong()
            .size(font_size::BODY)
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(4.0);
}

fn toggle_row(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
    ui.checkbox(value, label).changed()
}

fn preferences(ctx: &egui::Context, app: &mut App, look: &Look) -> Shown {
    let mut changed = false;
    let mut closed = modal(ctx, look, "preferences", 520.0, false, |ui| {
        if heading(ui, look, tr("Preferences"), "prefs") {
            ui.data_mut(|d| d.insert_temp(Id::unique("prefs-close"), true));
        }
        egui::ScrollArea::vertical()
            .id_salt("prefs-scroll")
            .max_height((ui.ctx().content_rect().height() - 180.0).max(200.0))
            .show(ui, |ui| {
                let s = &mut app.settings;
                section(ui, look, tr("Appearance"));
                ui.horizontal(|ui| {
                    for (a, label) in [
                        (Appearance::System, tr("System")),
                        (Appearance::Light, tr("Light")),
                        (Appearance::Dark, tr("Dark")),
                    ] {
                        if ui.selectable_label(s.appearance == a, label).clicked() {
                            s.appearance = a;
                            changed = true;
                        }
                    }
                });
                changed |= toggle_row(ui, tr("High contrast"), &mut s.high_contrast);
                changed |= toggle_row(ui, tr("Reduce motion"), &mut s.reduced_motion);
                if crate::chrome::NDL_CONTROLS_POSSIBLE {
                    let mut system = s.native_window_controls;
                    if toggle_row(ui, tr("Use the desktop's own window buttons"), &mut system) {
                        s.native_window_controls = system;
                        changed = true;
                    }
                }

                section(ui, look, tr("Language"));
                ui.horizontal(|ui| {
                    for (code, label) in
                        [("auto", tr("System")), ("es", "Español"), ("en", "English")]
                    {
                        if ui.selectable_label(s.language == code, label).clicked() {
                            s.language = code.to_owned();
                            i18n::set(Lang::from_setting(code));
                            changed = true;
                        }
                    }
                });

                section(ui, look, tr("Editor"));
                ui.horizontal(|ui| {
                    ui.label(tr("Font size"));
                    changed |= ui
                        .add(egui::Slider::new(
                            &mut s.font_size,
                            codigo_engine::settings::MIN_FONT_SIZE
                                ..=codigo_engine::settings::MAX_FONT_SIZE,
                        ))
                        .changed();
                });
                changed |= toggle_row(ui, tr("Word wrap"), &mut s.word_wrap);
                changed |= toggle_row(ui, tr("Line numbers"), &mut s.line_numbers);
                changed |= toggle_row(
                    ui,
                    tr("Highlight the current line"),
                    &mut s.highlight_active_line,
                );
                changed |= toggle_row(ui, tr("Show whitespace"), &mut s.show_whitespace);
                changed |= toggle_row(
                    ui,
                    tr("Close brackets and quotes automatically"),
                    &mut s.auto_close_pairs,
                );

                section(ui, look, tr("Saving"));
                changed |= toggle_row(
                    ui,
                    tr("Trim trailing whitespace on save"),
                    &mut s.trim_trailing_whitespace_on_save,
                );
                changed |= toggle_row(
                    ui,
                    tr("Insert a final newline on save"),
                    &mut s.insert_final_newline_on_save,
                );
                ui.horizontal(|ui| {
                    ui.label(tr("Auto save"));
                    for (a, label) in [
                        (AutoSave::Off, tr("Off")),
                        (AutoSave::OnFocusLoss, tr("When the window loses focus")),
                        (AutoSave::AfterDelay, tr("After a delay")),
                    ] {
                        if ui.selectable_label(s.auto_save == a, label).clicked() {
                            s.auto_save = a;
                            changed = true;
                        }
                    }
                });

                section(ui, look, tr("Updates"));
                changed |= toggle_row(
                    ui,
                    tr("Check for updates when Código starts"),
                    &mut s.check_updates,
                );
            });
    });
    if ctx
        .data_mut(|d| d.remove_temp::<bool>(Id::unique("prefs-close")))
        .unwrap_or(false)
    {
        closed = true;
    }
    if changed {
        app.save_settings();
    }
    if closed {
        (Overlay::None, true)
    } else {
        (Overlay::Preferences, false)
    }
}

fn shortcuts(ctx: &egui::Context, look: &Look, mut filter: String) -> Shown {
    let mac = cfg!(target_os = "macos");
    let p = look.palette;
    let mut closed = false;
    let dismissed = modal(ctx, look, "shortcuts", 560.0, false, |ui| {
        closed = heading(ui, look, tr("Keyboard shortcuts"), "keys");
        ui.add_space(8.0);
        cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Filter commands…"))
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY),
                );
            });
        ui.add_space(8.0);
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("keys-scroll")
            .max_height((ui.ctx().content_rect().height() - 220.0).max(200.0))
            .show(ui, |ui| {
                for category in [
                    Category::File,
                    Category::Edit,
                    Category::Selection,
                    Category::View,
                    Category::Go,
                    Category::Help,
                ] {
                    let rows: Vec<&Cmd> = Cmd::ALL
                        .iter()
                        .filter(|c| c.category() == category)
                        .filter(|c| {
                            needle.is_empty() || tr(c.label()).to_lowercase().contains(&needle)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    section(ui, look, tr(category.label()));
                    for cmd in rows {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr(cmd.label())).color(color(p.text_primary)));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(s) = cmd.shortcut() {
                                    ui.label(
                                        RichText::new(s.label(mac))
                                            .monospace()
                                            .color(color(p.text_secondary)),
                                    );
                                }
                            });
                        });
                    }
                }
            });
    });
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Shortcuts(filter), false)
    }
}

fn about_window(ctx: &egui::Context, look: &Look) -> Shown {
    let version = trf("Version {}", &[env!("CARGO_PKG_VERSION")]);
    let links = [
        Link {
            label: tr("Website"),
            url: "https://getnodalia.com/codigo",
        },
        Link {
            label: tr("Source code"),
            url: "https://github.com/getnodalia/codigo",
        },
        Link {
            label: tr("Report a problem"),
            url: "https://github.com/getnodalia/codigo/issues",
        },
    ];
    let footer = format!(
        "MIT OR Apache-2.0 · © Nodalia · {}",
        tr(crate::icon::CREDIT)
    );
    let info = Info {
        name: crate::app::NAME,
        version: &version,
        tagline: tr("A code editor for any language, with AI built in, native and fast."),
        suite: tr("Part of Nodalia · Engineering"),
        links: &links,
        footer: &footer,
    };
    let mut closed = false;
    let dismissed = modal(ctx, look, "about", about::measures::WIDTH, false, |ui| {
        closed = heading(ui, look, tr("About Código"), "about");
        let page = about::page(ui, look, &info, |ui, rect| {
            let texture = crate::icon::texture(ui.ctx());
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        });
        if let Some(i) = page.picked {
            if let Some(link) = links.get(i) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(link.url));
            }
        }
    });
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::About, false)
    }
}

fn confirm(ctx: &egui::Context, app: &mut App, look: &Look, c: Confirm) -> Shown {
    let (title, body, names): (&str, String, Vec<usize>) = match &c {
        Confirm::Close(indices) => {
            let dirty: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|i| app.tabs.get(*i).is_some_and(|t| t.doc.is_dirty()))
                .collect();
            (
                tr("Unsaved changes"),
                tr("These files have changes that are not saved.").to_owned(),
                dirty,
            )
        }
        Confirm::Quit => (
            tr("Unsaved changes"),
            tr("These files have changes that are not saved.").to_owned(),
            app.dirty_tabs(),
        ),
        Confirm::Conflict(i) => (
            tr("The file changed on disk"),
            tr("Another program changed this file while you were editing it.").to_owned(),
            vec![*i],
        ),
    };
    enum Pick {
        Save,
        Discard,
        Cancel,
        None,
    }
    let mut pick = Pick::None;
    let conflict = matches!(c, Confirm::Conflict(_));
    let dismissed = modal(ctx, look, "confirm", 460.0, false, |ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .size(font_size::TITLE)
                .color(color(look.palette.text_primary)),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(body).color(color(look.palette.text_secondary)));
        ui.add_space(8.0);
        for i in &names {
            if let Some(name) = app.tabs.get(*i).and_then(|t| t.doc.file_name()) {
                ui.label(
                    RichText::new(format!("• {name}")).color(color(look.palette.text_primary)),
                );
            } else if app.tabs.get(*i).is_some() {
                ui.label(
                    RichText::new(format!("• {}", tr("Untitled")))
                        .color(color(look.palette.text_primary)),
                );
            }
        }
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save_label = if conflict {
                tr("Overwrite")
            } else {
                tr("Save")
            };
            let discard_label = if conflict {
                tr("Reload from disk")
            } else {
                tr("Don't save")
            };
            if cr::primary_button(ui, look, save_label, true).clicked() {
                pick = Pick::Save;
            }
            if cr::secondary_button_min(ui, look, discard_label, true, 110.0).clicked() {
                pick = Pick::Discard;
            }
            if cr::secondary_button(ui, look, tr("Cancel"), true).clicked() {
                pick = Pick::Cancel;
            }
        });
    });
    if dismissed {
        pick = Pick::Cancel;
    }
    match (pick, c) {
        (Pick::None, c) => (Overlay::Confirm(c), false),
        (Pick::Cancel, _) => (Overlay::None, true),
        (Pick::Save, Confirm::Conflict(i)) => {
            if let Some(tab) = app.tabs.get_mut(i) {
                tab.doc.accept_disk_state();
                tab.doc.conflict = false;
            }
            app.save_tab(i);
            (Overlay::None, true)
        }
        (Pick::Discard, Confirm::Conflict(i)) => {
            if let Some(tab) = app.tabs.get_mut(i) {
                let _ = tab.doc.reload();
                tab.doc.conflict = false;
            }
            (Overlay::None, true)
        }
        (Pick::Save, Confirm::Close(indices)) => {
            let ok = names.iter().all(|i| app.save_tab(*i));
            if ok {
                let mut sorted = indices;
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                for i in sorted {
                    app.close_tab_now(i);
                }
            }
            (Overlay::None, true)
        }
        (Pick::Discard, Confirm::Close(indices)) => {
            let mut sorted = indices;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for i in sorted {
                app.close_tab_now(i);
            }
            (Overlay::None, true)
        }
        (Pick::Save, Confirm::Quit) => {
            if app.save_all() {
                app.quit_now(ctx);
            }
            (Overlay::None, true)
        }
        (Pick::Discard, Confirm::Quit) => {
            app.quit_now(ctx);
            (Overlay::None, true)
        }
    }
}
