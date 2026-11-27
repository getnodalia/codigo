// SPDX-License-Identifier: MIT OR Apache-2.0
//! The terminal panel under the editor: shells and the run console are the same thing, a program
//! in a pseudo-terminal, drawn here as a grid of cells.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use codigo_engine::{tasks, Cmd, TaskKind};
use codigo_term::keys::{self, Key as TKey, Mods};
use codigo_term::links::{self, FileRef};
use codigo_term::{Color, Spec, Status, Terminal, Waker};
use egui::{pos2, vec2, Event, FontId, Key, Rect, Sense, Stroke, Ui};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::overlays::Overlay;
use crate::widgets::small_icon_button;

const HEADER_H: f32 = 30.0;
pub const MIN_HEIGHT: f32 = 120.0;
const DEFAULT_HEIGHT: f32 = 260.0;

type Cell = (u16, u16);

pub struct TermTab {
    pub title: String,
    pub term: Terminal,
    /// A task (run, build, test) rather than an interactive shell.
    pub task: bool,
    selection: Option<(Cell, Cell)>,
}

pub struct TermPanel {
    pub open: bool,
    pub height: f32,
    pub tabs: Vec<TermTab>,
    pub active: usize,
    /// The panel asked for the keyboard on the next frame.
    pub want_focus: bool,
    /// It has the keyboard, as of the last frame.
    pub focused: bool,
}

impl Default for TermPanel {
    fn default() -> Self {
        TermPanel {
            open: false,
            height: DEFAULT_HEIGHT,
            tabs: Vec::new(),
            active: 0,
            want_focus: false,
            focused: false,
        }
    }
}

fn waker(ctx: &egui::Context) -> Waker {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

fn work_dir(app: &App) -> PathBuf {
    app.project
        .as_ref()
        .map(|p| p.root.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn start(app: &mut App, ctx: &egui::Context, spec: Spec, title: String, task: bool) {
    match Terminal::spawn(&spec, 24, 80, Some(waker(ctx))) {
        Ok(term) => {
            app.term.tabs.push(TermTab {
                title,
                term,
                task,
                selection: None,
            });
            app.term.active = app.term.tabs.len() - 1;
            app.term.open = true;
            app.term.want_focus = !task;
        }
        Err(e) => {
            tracing::warn!("terminal: {e}");
            app.toast(trf(
                "The terminal could not be started: {}",
                &[&e.to_string()],
            ));
        }
    }
}

pub fn new_shell(app: &mut App, ctx: &egui::Context) {
    let spec = codigo_term::shell_command(work_dir(app));
    let n = app.term.tabs.iter().filter(|t| !t.task).count() + 1;
    start(app, ctx, spec, trf("Terminal {}", &[&n.to_string()]), false);
}

pub fn toggle(app: &mut App, ctx: &egui::Context) {
    if app.term.open {
        app.term.open = false;
        app.term.focused = false;
        app.focus_editor = true;
    } else if app.term.tabs.is_empty() {
        new_shell(app, ctx);
    } else {
        app.term.open = true;
        app.term.want_focus = true;
    }
}

pub fn close_active(app: &mut App) {
    if app.term.active < app.term.tabs.len() {
        app.term.tabs.remove(app.term.active);
    }
    app.term.active = app.term.active.min(app.term.tabs.len().saturating_sub(1));
    if app.term.tabs.is_empty() {
        app.term.open = false;
        app.term.focused = false;
        app.focus_editor = true;
    }
}

/// Run the project's task of this kind in the console.
pub fn run_task(app: &mut App, ctx: &egui::Context, kind: TaskKind) {
    let Some(root) = crate::setup_ui::work_root(app) else {
        app.toast(tr("Open a folder to run its tasks."));
        return;
    };
    let active = app
        .active_doc()
        .and_then(|d| d.path())
        .map(Path::to_path_buf);
    let Some(task) = tasks::resolve(&root, active.as_deref(), kind) else {
        // Nothing recognised: ask once and remember the answer.
        app.overlay = Overlay::RunSetup(crate::setup_ui::RunSetup::new(kind));
        return;
    };
    // Running the same task again replaces its console rather than stacking another.
    if let Some(i) = app
        .term
        .tabs
        .iter()
        .position(|t| t.task && t.title == task.label)
    {
        app.term.tabs.remove(i);
    }
    let spec = Spec {
        program: task.program.clone(),
        args: task.args.clone(),
        cwd: task.cwd.clone().unwrap_or(root),
        env: Vec::new(),
    };
    let label = task.label.clone();
    // Save what is being edited first, so the program runs what is on screen.
    app.run(ctx, Cmd::SaveAll);
    start(app, ctx, spec, label, true);
}

/// Whether a task (run, build, test) is still going.
pub fn task_running(app: &mut App) -> bool {
    app.term
        .tabs
        .iter_mut()
        .any(|t| t.task && t.term.status() == codigo_term::Status::Running)
}

/// Stop every task that is still going.
pub fn stop(app: &mut App) {
    for tab in app.term.tabs.iter_mut().filter(|t| t.task) {
        if tab.term.status() == codigo_term::Status::Running {
            tab.term.kill();
        }
    }
}

// --- Colours -----------------------------------------------------------------------------------

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// The 16 base colours come from the palette, so a theme change recolours the terminal too.
fn ansi16(look: &Look, n: u8) -> egui::Color32 {
    let p = &look.palette;
    let bright = n >= 8;
    let base = match n % 8 {
        0 => color(p.text_tertiary),
        1 => color(p.status_danger_fg),
        2 => color(p.status_success_fg),
        3 => color(p.status_warning_fg),
        4 => color(p.status_info_fg),
        5 => mix(color(p.accent_text), color(p.status_danger_fg), 0.5),
        6 => color(p.accent_text),
        _ => color(p.text_secondary),
    };
    if bright {
        mix(base, color(p.text_primary), 0.3)
    } else {
        base
    }
}

fn indexed(look: &Look, i: u8) -> egui::Color32 {
    match i {
        0..=15 => ansi16(look, i),
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            egui::Color32::from_rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            egui::Color32::from_rgb(g, g, g)
        }
    }
}

fn resolve(look: &Look, c: Color, default: egui::Color32) -> egui::Color32 {
    match c {
        Color::Default => default,
        Color::Idx(i) => indexed(look, i),
        Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}

// --- Keyboard ----------------------------------------------------------------------------------

fn term_key(key: Key) -> Option<TKey> {
    Some(match key {
        Key::Enter => TKey::Enter,
        Key::Backspace => TKey::Backspace,
        Key::Tab => TKey::Tab,
        Key::Escape => TKey::Escape,
        Key::ArrowUp => TKey::Up,
        Key::ArrowDown => TKey::Down,
        Key::ArrowLeft => TKey::Left,
        Key::ArrowRight => TKey::Right,
        Key::Home => TKey::Home,
        Key::End => TKey::End,
        Key::PageUp => TKey::PageUp,
        Key::PageDown => TKey::PageDown,
        Key::Insert => TKey::Insert,
        Key::Delete => TKey::Delete,
        Key::F1 => TKey::F(1),
        Key::F2 => TKey::F(2),
        Key::F3 => TKey::F(3),
        Key::F4 => TKey::F(4),
        Key::F5 => TKey::F(5),
        Key::F6 => TKey::F(6),
        Key::F7 => TKey::F(7),
        Key::F8 => TKey::F(8),
        Key::F9 => TKey::F(9),
        Key::F10 => TKey::F(10),
        Key::F11 => TKey::F(11),
        Key::F12 => TKey::F(12),
        _ => return None,
    })
}

/// A letter or digit key, for chords with Ctrl or Alt (which produce no text event).
fn char_of(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_none() && c.is_ascii_alphanumeric() {
        Some(c.to_ascii_lowercase())
    } else {
        None
    }
}

/// Commands that keep working while the terminal has the keyboard; the rest of the keys belong
/// to the program in it.
const APP_KEYS: &[Cmd] = &[
    Cmd::ToggleTerminal,
    Cmd::NewTerminal,
    Cmd::CommandPalette,
    Cmd::QuickOpen,
    Cmd::ToggleSidebar,
    Cmd::RunProject,
    Cmd::BuildProject,
    Cmd::StopRun,
    Cmd::FocusExplorer,
];

fn is_app_shortcut(key: Key, m: &egui::Modifiers) -> bool {
    APP_KEYS.iter().any(|cmd| {
        cmd.shortcut().is_some_and(|s| {
            crate::app::to_egui_key(s.key) == Some(key)
                && s.shift == m.shift
                && s.alt == m.alt
                && s.primary == m.command
        })
    })
}

/// Route keyboard events to the focused terminal. Run before the app's own shortcuts.
pub fn input(app: &mut App, ctx: &egui::Context) {
    if !app.term.focused || !app.term.open {
        return;
    }
    let Some(tab) = app.term.tabs.get_mut(app.term.active) else {
        return;
    };
    let mut out: Vec<u8> = Vec::new();
    let mut copied: Option<String> = None;
    let app_cursor = tab.term.screen(|s| s.application_cursor());
    let bracketed = tab.term.screen(|s| s.bracketed_paste());
    ctx.input_mut(|i| {
        let events = std::mem::take(&mut i.events);
        let mut kept = Vec::new();
        let alt_down = i.modifiers.alt;
        for event in events {
            match &event {
                Event::Text(t) if !alt_down => {
                    out.extend(
                        t.chars()
                            .filter(|c| !c.is_control())
                            .collect::<String>()
                            .bytes(),
                    );
                }
                Event::Text(_) => {}
                Event::Paste(text) => out.extend(keys::paste(text, bracketed)),
                Event::Copy => match tab
                    .selection
                    .take()
                    .map(|(a, b)| tab.term.screen(|s| selection_text(s, a, b)))
                {
                    Some(text) => copied = Some(text),
                    None => out.push(3),
                },
                Event::Cut => out.push(0x18),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if is_app_shortcut(*key, modifiers) {
                        kept.push(event);
                        continue;
                    }
                    let mods = Mods {
                        ctrl: modifiers.ctrl || modifiers.command,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                    };
                    if let Some(k) = term_key(*key) {
                        out.extend(keys::encode(k, mods, app_cursor));
                    } else if mods.ctrl || mods.alt {
                        if let Some(c) = char_of(*key) {
                            out.extend(keys::encode(TKey::Char(c), mods, app_cursor));
                        }
                    }
                }
                Event::Key { .. } => {}
                _ => kept.push(event),
            }
        }
        i.events = kept;
    });
    if let Some(text) = copied {
        ctx.copy_text(text);
    }
    if !out.is_empty() {
        tab.selection = None;
        tab.term.write(&out);
    }
}

fn selection_text(screen: &codigo_term::Screen, a: Cell, b: Cell) -> String {
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    screen.contents_between(start.0, start.1, end.0, end.1.saturating_add(1))
}

// --- Drawing -----------------------------------------------------------------------------------

/// Draw the panel into `ui`, which is the terminal island.
pub fn show(ui: &mut Ui, app: &mut App, look: &Look) {
    let p = &look.palette;
    let font_size = app.editor_options().font_size;
    let ctx = ui.ctx().clone();
    let mut close_tab = None;
    let mut new_shell_clicked = false;
    let mut hide = false;
    let mut switch = None;
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, color(p.border_default)),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let active = app.term.active;
            for (i, tab) in app.term.tabs.iter_mut().enumerate() {
                let status = tab.term.status();
                let (dot, tip) = match status {
                    Status::Running => (color(p.status_success_fg), tr("Running")),
                    Status::Exited(0) => (color(p.text_tertiary), tr("Finished")),
                    Status::Exited(_) => (color(p.status_danger_fg), tr("Failed")),
                };
                let text = match status {
                    Status::Exited(code) if tab.task && code != 0 => {
                        format!("{} ({code})", tab.title)
                    }
                    _ => tab.title.clone(),
                };
                let galley = ui.painter().layout_no_wrap(
                    text,
                    FontId::proportional(nodalia_theme::font_size::BODY),
                    if i == active {
                        color(p.text_primary)
                    } else {
                        color(p.text_secondary)
                    },
                );
                let (rect, response) =
                    ui.allocate_exact_size(vec2(galley.size().x + 50.0, 24.0), Sense::click());
                if i == active {
                    ui.painter()
                        .rect_filled(rect, 8.0, color(p.control_bg_hover));
                } else if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        8.0,
                        color(p.control_bg_hover).gamma_multiply(0.5),
                    );
                }
                ui.painter()
                    .circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, dot);
                ui.painter().galley(
                    pos2(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0),
                    galley.clone(),
                    color(p.text_primary),
                );
                let x = Rect::from_center_size(
                    pos2(rect.right() - 13.0, rect.center().y),
                    vec2(16.0, 16.0),
                );
                if let Some(icon) = nodalia_theme::icons::get("close") {
                    nodalia_theme::icons::paint(
                        ui.painter(),
                        x.shrink(3.0),
                        icon,
                        nodalia_theme::icons::Style::bold(color(p.text_tertiary)),
                    );
                }
                let over_x = response
                    .hover_pos()
                    .is_some_and(|pos| x.expand(2.0).contains(pos));
                let response = response.on_hover_text(tip);
                if response.middle_clicked() || (response.clicked() && over_x) {
                    close_tab = Some(i);
                } else if response.clicked() {
                    switch = Some(i);
                }
            }
            if small_icon_button(ui, look, "plus", tr("New terminal"), false).clicked() {
                new_shell_clicked = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if small_icon_button(ui, look, "chevron-down", tr("Hide the terminal"), false)
                    .clicked()
                {
                    hide = true;
                }
            });
        },
    );
    if let Some(i) = switch {
        app.term.active = i;
        app.term.want_focus = true;
    }
    if new_shell_clicked {
        new_shell(app, &ctx);
    }
    if let Some(i) = close_tab {
        app.term.active = i;
        close_active(app);
    }
    if hide {
        toggle(app, &ctx);
    }

    let area = ui.available_rect_before_wrap();
    let font = FontId::monospace(font_size);
    let cw = ui
        .painter()
        .layout_no_wrap("M".into(), font.clone(), egui::Color32::WHITE)
        .size()
        .x
        .max(1.0);
    let row_h = (font_size * 1.35).round().max(8.0);
    let inner = area.shrink2(vec2(10.0, 4.0));
    let cols = ((inner.width() / cw).floor() as u16).max(2);
    let rows = ((inner.height() / row_h).floor() as u16).max(1);

    let id = ui.scope_id().with("terminal-screen");
    let response = ui.interact(area, id, Sense::click_and_drag());
    ctx.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });
    if app.term.want_focus {
        response.request_focus();
        app.term.want_focus = false;
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    app.term.focused = ctx.memory(|m| m.has_focus(id));
    let focused = app.term.focused;

    let root = app.project.as_ref().map(|p| p.root.clone());
    let mut open_ref: Option<FileRef> = None;
    let Some(tab) = app.term.tabs.get_mut(app.term.active) else {
        return;
    };
    tab.term.resize(rows, cols);

    // Scrolling and selecting.
    if response.hovered() {
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            tab.term.scroll((scroll / row_h).round() as i32);
        }
    }
    let cell_at = |pos: egui::Pos2| -> Cell {
        let c = (((pos.x - inner.left()) / cw).floor().max(0.0) as u16).min(cols - 1);
        let r = (((pos.y - inner.top()) / row_h).floor().max(0.0) as u16).min(rows - 1);
        (r, c)
    };
    if response.drag_started() {
        tab.selection = response
            .interact_pointer_pos()
            .map(|p| (cell_at(p), cell_at(p)));
    } else if response.dragged() {
        if let (Some((start, _)), Some(pos)) = (tab.selection, response.interact_pointer_pos()) {
            tab.selection = Some((start, cell_at(pos)));
        }
    } else if response.clicked() {
        tab.selection = None;
    }

    // A click on `path:line:column` opens that place; the pointer says it can be clicked.
    let reference_at = |tab: &TermTab, pos: egui::Pos2| -> Option<FileRef> {
        let (row, col) = cell_at(pos);
        tab.term.screen(|s| {
            let line: Vec<char> = (0..cols)
                .map(|c| {
                    s.cell(row, c)
                        .and_then(|cell| cell.contents().chars().next())
                        .unwrap_or(' ')
                })
                .collect();
            links::file_ref_at(&line, usize::from(col))
        })
    };
    if let Some(pos) = response.hover_pos() {
        if reference_at(tab, pos).is_some_and(|r| resolve_ref(root.as_deref(), &r).is_some()) {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            open_ref = reference_at(tab, pos);
        }
    }

    let background = color(p.surface_panel);
    let default_fg = color(p.text_primary);
    let painter = ui.painter_at(area);
    let selection = tab
        .selection
        .map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
    let history = tab.term.screen(|s| s.scrollback());
    tab.term.screen(|screen| {
        let (cursor_row, cursor_col) = screen.cursor_position();
        for row in 0..rows {
            let y = inner.top() + f32::from(row) * row_h;
            let mut run = String::new();
            let mut run_start = 0u16;
            let mut run_ink = default_fg;
            let flush = |run: &mut String, start: u16, ink: egui::Color32| {
                if !run.is_empty() {
                    painter.text(
                        pos2(inner.left() + f32::from(start) * cw, y),
                        egui::Align2::LEFT_TOP,
                        run.as_str(),
                        font.clone(),
                        ink,
                    );
                    run.clear();
                }
            };
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = resolve(look, cell.fgcolor(), default_fg);
                let mut bg = match cell.bgcolor() {
                    Color::Default => None,
                    c => Some(resolve(look, c, background)),
                };
                if cell.inverse() {
                    let swapped = bg.unwrap_or(background);
                    bg = Some(fg);
                    fg = swapped;
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.6);
                }
                let selected = selection.is_some_and(|(a, b)| (row, col) >= a && (row, col) <= b);
                if selected {
                    bg = Some(color(p.accent_default).gamma_multiply(0.45));
                }
                let width = if cell.is_wide() { 2.0 } else { 1.0 };
                if let Some(bg) = bg {
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(inner.left() + f32::from(col) * cw, y),
                            vec2(cw * width, row_h),
                        ),
                        0.0,
                        bg,
                    );
                }
                let shows_cursor = focused
                    && history == 0
                    && !screen.hide_cursor()
                    && (row, col) == (cursor_row, cursor_col);
                if shows_cursor {
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(inner.left() + f32::from(col) * cw, y),
                            vec2(cw, row_h),
                        ),
                        0.0,
                        color(p.accent_default),
                    );
                    fg = background;
                }
                let text = cell.contents();
                let text = if text.is_empty() { " " } else { text };
                let plain = text == " ";
                if cell.is_wide() || fg != run_ink || cell.bold() || cell.underline() {
                    flush(&mut run, run_start, run_ink);
                }
                if cell.is_wide() || cell.bold() || cell.underline() {
                    painter.text(
                        pos2(inner.left() + f32::from(col) * cw, y),
                        egui::Align2::LEFT_TOP,
                        text,
                        font.clone(),
                        fg,
                    );
                    if cell.underline() {
                        let base = y + row_h - 2.0;
                        painter.line_segment(
                            [
                                pos2(inner.left() + f32::from(col) * cw, base),
                                pos2(inner.left() + f32::from(col) * cw + cw * width, base),
                            ],
                            Stroke::new(1.0, fg),
                        );
                    }
                    continue;
                }
                if run.is_empty() {
                    if plain {
                        continue;
                    }
                    run_start = col;
                    run_ink = fg;
                } else if fg != run_ink {
                    run_start = col;
                    run_ink = fg;
                }
                // Fill any gap with spaces so the run stays aligned to the grid.
                let have = run.chars().count() as u16;
                for _ in have..col.saturating_sub(run_start) {
                    run.push(' ');
                }
                run.push_str(text);
            }
            flush(&mut run, run_start, run_ink);
        }
    });
    if !focused {
        ui.painter().text(
            pos2(area.right() - 12.0, area.bottom() - 8.0),
            egui::Align2::RIGHT_BOTTOM,
            tr("Click to type"),
            FontId::proportional(nodalia_theme::font_size::SMALL),
            color(p.text_tertiary),
        );
    }
    if let Some(reference) = open_ref {
        if let Some(path) = resolve_ref(root.as_deref(), &reference) {
            app.open_path(&path, Some(reference.line.saturating_sub(1)), true);
        }
    }
}

/// The file a reference names, relative to the project folder, when it exists.
fn resolve_ref(root: Option<&std::path::Path>, reference: &FileRef) -> Option<PathBuf> {
    let named = PathBuf::from(&reference.path);
    let path = if named.is_absolute() {
        named
    } else {
        root?.join(named)
    };
    path.is_file().then_some(path)
}
