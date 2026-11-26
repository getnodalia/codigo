// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a language server answers beyond diagnostics: go to definition, hover text and completion.
//! The request goes out on a command, the answer is polled each frame, and the result is either a
//! jump or a popup next to the caret.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use codigo_engine::{
    char_in_line, edits_for, Answer, CompletionItem, DocId, FileChange, Place, Query, QueryError,
    QueryKind,
};
use codigo_text::{Edit, Indent};
use egui::{
    pos2, vec2, Align2, CornerRadius, FontId, Key, Modifiers, Order, RichText, ScrollArea, Stroke,
};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};

/// At most this many rows of the completion list are shown at once.
const ROWS: usize = 10;

/// A request in flight, and what it was asked about, so a late answer is not applied to text the
/// person has since changed.
pub struct InFlight {
    query: Query,
    doc: DocId,
    revision: u64,
    /// For completion: where the word being completed starts.
    anchor: usize,
    /// A request the person did not ask for (typing) reports nothing when it fails.
    quiet: bool,
}

pub struct Completion {
    pub items: Vec<CompletionItem>,
    pub anchor: usize,
    pub selected: usize,
    pub doc: DocId,
}

#[derive(Default)]
pub enum Popup {
    #[default]
    None,
    Hover {
        text: String,
        doc: DocId,
        state: (u64, usize),
    },
    Completion(Completion),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Start a request at the caret of the active file.
pub fn ask(app: &mut App, kind: QueryKind, quiet: bool) {
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let buffer = tab.doc.buffer();
    if buffer.selections().len() > 1 {
        return;
    }
    let head = buffer.primary().head;
    let mut anchor = head;
    while anchor > 0 && buffer.slice(anchor - 1..anchor).chars().all(is_word) {
        anchor -= 1;
    }
    let (doc, revision) = (tab.doc.id(), buffer.revision());
    let result = app.lsp.query(&tab.doc, head, kind);
    match result {
        Ok(query) => {
            app.query = Some(InFlight {
                query,
                doc,
                revision,
                anchor,
                quiet,
            });
        }
        Err(e) if !quiet => app.toast(match e {
            QueryError::NoServer => tr("There is no language server for this file."),
            QueryError::NotReady => tr("The language server is not ready yet."),
        }),
        Err(_) => {}
    }
}

/// Take the answer if it arrived. Called once a frame.
pub fn poll(app: &mut App) {
    let Some(flight) = app.query.as_ref() else {
        return;
    };
    let Some(result) = flight.query.poll() else {
        return;
    };
    let Some(flight) = app.query.take() else {
        return;
    };
    let current = app
        .tabs
        .get(app.active)
        .map(|t| (t.doc.id(), t.doc.buffer().revision()));
    let fresh = current == Some((flight.doc, flight.revision));
    match result {
        Err(message) => {
            if !flight.quiet {
                tracing::debug!("language server: {message}");
                app.toast(tr("The language server could not answer."));
            }
        }
        Ok(Answer::Places(places)) => {
            let Some(place) = places.into_iter().next() else {
                if !flight.quiet {
                    app.toast(tr("No definition found."));
                }
                return;
            };
            jump(app, &place.path, place.line, place.character);
        }
        Ok(Answer::References(places)) => {
            if places.is_empty() {
                app.toast(tr("No references found."));
                return;
            }
            let word = word_at(app, flight.anchor);
            app.overlay = Overlay::References(references_view(app, word, places));
        }
        Ok(Answer::Edits(changes)) => {
            if !fresh {
                app.toast(tr(
                    "The text changed while the server was working. Try again.",
                ));
                return;
            }
            apply_changes(app, changes, flight.query.kind == QueryKind::Format);
        }
        Ok(Answer::Hover(text)) if fresh => match (text, app.tabs.get(app.active)) {
            (Some(text), Some(tab)) => {
                let b = tab.doc.buffer();
                app.popup = Popup::Hover {
                    text,
                    doc: flight.doc,
                    state: (b.revision(), b.primary().head),
                };
            }
            _ => {
                if !flight.quiet {
                    app.toast(tr("No information here."));
                }
            }
        },
        Ok(Answer::Completion(items)) if fresh && !items.is_empty() => {
            app.popup = Popup::Completion(Completion {
                items,
                anchor: flight.anchor,
                selected: 0,
                doc: flight.doc,
            });
        }
        Ok(_) => {}
    }
}

/// Open `path` and put the caret at a server's line and UTF-16 column.
fn jump(app: &mut App, path: &Path, line: usize, character: u32) {
    if !app.open_path(path, Some(line), true) {
        return;
    }
    if let Some(tab) = app.tabs.get_mut(app.active) {
        let col = char_in_line(&tab.doc, line, character);
        tab.doc.navigate(|b| {
            let line = line.min(b.line_count().saturating_sub(1));
            b.set_cursor(b.line_start(line) + col);
        });
        tab.view.reveal = true;
    }
}

fn tab_for(app: &App, path: &Path) -> Option<usize> {
    let want = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    app.tabs.iter().position(|t| {
        t.doc
            .path()
            .is_some_and(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()) == want)
    })
}

/// Make a server's edits. Files that are not open are opened first, so the changes show as
/// unsaved edits that can be reviewed, undone or saved.
fn apply_changes(app: &mut App, changes: Vec<FileChange>, format: bool) {
    let back = app.tabs.get(app.active).map(|t| t.doc.id());
    let (mut places, mut files, mut skipped) = (0usize, 0usize, 0usize);
    for change in changes {
        if change.edits.is_empty() {
            continue;
        }
        if tab_for(app, &change.path).is_none() {
            app.open_path(&change.path, None, false);
        }
        let Some(i) = tab_for(app, &change.path) else {
            skipped += 1;
            continue;
        };
        let Some(tab) = app.tabs.get_mut(i) else {
            continue;
        };
        let edits = edits_for(&tab.doc, &change.edits);
        let count = edits.len();
        let mut applied = false;
        tab.doc.apply(|b| match b.apply_edits(edits) {
            Some(changes) => {
                applied = true;
                changes
            }
            None => Vec::new(),
        });
        if applied {
            tab.view.reveal = false;
            places += count;
            files += 1;
        } else {
            skipped += 1;
        }
    }
    if let Some(id) = back {
        if let Some(i) = app.tabs.iter().position(|t| t.doc.id() == id) {
            app.active = i;
        }
    }
    let message = if skipped > 0 && files == 0 {
        tr("The server's changes could not be applied.").to_owned()
    } else if format && files == 0 {
        tr("Already formatted.").to_owned()
    } else if format {
        tr("Formatted.").to_owned()
    } else if files == 0 {
        tr("Nothing to change.").to_owned()
    } else {
        trf(
            "Changed {} places in {} files.",
            &[&places.to_string(), &files.to_string()],
        )
    };
    app.toast(message);
}

/// Format the active file with its language server.
pub fn format_document(app: &mut App) {
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let (size, spaces) = match tab.doc.buffer().indent {
        Indent::Tabs => (
            u32::try_from(tab.doc.buffer().tab_width).unwrap_or(4),
            false,
        ),
        Indent::Spaces(n) => (u32::try_from(n).unwrap_or(4), true),
    };
    let (doc, revision) = (tab.doc.id(), tab.doc.buffer().revision());
    let result = app.lsp.format(&tab.doc, size, spaces);
    match result {
        Ok(query) => {
            app.query = Some(InFlight {
                query,
                doc,
                revision,
                anchor: 0,
                quiet: false,
            });
        }
        Err(QueryError::NoServer) => app.toast(tr("There is no language server for this file.")),
        Err(QueryError::NotReady) => app.toast(tr("The language server is not ready yet.")),
    }
}

/// The identifier that touches char offset `at` of the active file; empty when there is none.
fn word_at(app: &App, at: usize) -> String {
    let Some(tab) = app.tabs.get(app.active) else {
        return String::new();
    };
    let b = tab.doc.buffer();
    let (mut start, mut end) = (at.min(b.len_chars()), at.min(b.len_chars()));
    while start > 0 && b.slice(start - 1..start).chars().all(is_word) {
        start -= 1;
    }
    while end < b.len_chars() && b.slice(end..end + 1).chars().all(is_word) {
        end += 1;
    }
    b.slice(start..end)
}

pub struct RenameSetup {
    name: String,
    old: String,
    doc: DocId,
    at: usize,
    focus: bool,
}

/// Ask for the new name of the symbol under the caret.
pub fn open_rename(app: &mut App) {
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let at = tab.doc.buffer().primary().head;
    let doc = tab.doc.id();
    let old = word_at(app, at);
    if old.is_empty() {
        app.toast(tr("Put the caret on a name to rename it."));
        return;
    }
    app.overlay = Overlay::Rename(RenameSetup {
        name: old.clone(),
        old,
        doc,
        at,
        focus: true,
    });
}

fn start_rename(app: &mut App, setup: &RenameSetup) {
    let Some(tab) = app.tabs.get(app.active).filter(|t| t.doc.id() == setup.doc) else {
        return;
    };
    let revision = tab.doc.buffer().revision();
    let result = app.lsp.rename(&tab.doc, setup.at, setup.name.trim());
    match result {
        Ok(query) => {
            app.query = Some(InFlight {
                query,
                doc: setup.doc,
                revision,
                anchor: setup.at,
                quiet: false,
            });
        }
        Err(QueryError::NoServer) => app.toast(tr("There is no language server for this file.")),
        Err(QueryError::NotReady) => app.toast(tr("The language server is not ready yet.")),
    }
}

pub fn rename_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut setup: RenameSetup,
) -> Shown {
    let mut closed = false;
    let mut go = false;
    let dismissed = modal(ctx, look, "rename", 440.0, true, |ui| {
        closed = heading(ui, look, tr("Rename symbol"), "rename");
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut setup.name)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("New name"))
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::LARGE)),
                )
            })
            .inner;
        if setup.focus {
            response.request_focus();
            // Select the old name so typing replaces it.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(setup.name.chars().count()),
                    )));
                state.store(ui.ctx(), response.id);
            }
            setup.focus = false;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            go = true;
        }
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    let name = setup.name.trim();
    if go && !name.is_empty() && name != setup.old && !name.contains(char::is_whitespace) {
        start_rename(app, &setup);
        return (Overlay::None, true);
    }
    (Overlay::Rename(setup), false)
}

pub struct RefItem {
    path: PathBuf,
    line: usize,
    character: u32,
    preview: String,
}

pub struct RefsView {
    word: String,
    items: Vec<RefItem>,
    root: Option<PathBuf>,
}

fn references_view(app: &App, word: String, places: Vec<Place>) -> RefsView {
    // Read each file once; a file that is open is read from its buffer, which may be unsaved.
    let mut lines: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let items = places
        .into_iter()
        .map(|p| {
            let all = lines.entry(p.path.clone()).or_insert_with(|| {
                match tab_for(app, &p.path).and_then(|i| app.tabs.get(i)) {
                    Some(tab) => tab.doc.buffer().text().lines().map(str::to_owned).collect(),
                    None => read_lines(&p.path),
                }
            });
            let preview = all
                .get(p.line)
                .map(|l| l.trim().chars().take(160).collect())
                .unwrap_or_default();
            RefItem {
                path: p.path,
                line: p.line,
                character: p.character,
                preview,
            }
        })
        .collect();
    RefsView {
        word,
        items,
        root: app.project.as_ref().map(|p| p.root.clone()),
    }
}

/// The lines of a file, for previews. A file too big to be source is not read.
fn read_lines(path: &Path) -> Vec<String> {
    const MAX: u64 = 2_000_000;
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX) {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .map(|t| t.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn references_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    view: RefsView,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut pick: Option<usize> = None;
    let dismissed = modal(ctx, look, "references", 720.0, true, |ui| {
        let title = trf(
            "{} references to “{}”",
            &[&view.items.len().to_string(), &view.word],
        );
        closed = heading(ui, look, &title, "references");
        ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
            for (n, item) in view.items.iter().enumerate() {
                let shown = view
                    .root
                    .as_ref()
                    .and_then(|r| item.path.strip_prefix(r).ok())
                    .unwrap_or(&item.path);
                let head = format!("{}:{}", shown.display(), item.line + 1);
                let response = ui
                    .vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(head)
                                .font(FontId::proportional(12.0))
                                .color(color(p.text_secondary)),
                        );
                        ui.label(
                            RichText::new(&item.preview)
                                .font(FontId::monospace(13.0))
                                .color(color(p.text_primary)),
                        );
                    })
                    .response
                    .interact(egui::Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(
                        response.rect.expand(2.0),
                        CornerRadius::same(6),
                        color(p.accent_default).gamma_multiply(0.12),
                    );
                }
                if response.clicked() {
                    pick = Some(n);
                }
                ui.add_space(4.0);
            }
        });
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if let Some(item) = pick.and_then(|n| view.items.get(n)) {
        jump(app, &item.path, item.line, item.character);
        app.focus_editor = true;
        return (Overlay::None, true);
    }
    (Overlay::References(view), false)
}

/// The items still matching what has been typed since the word began.
pub fn visible<'a>(c: &'a Completion, prefix: &str) -> Vec<&'a CompletionItem> {
    let want = prefix.to_lowercase();
    c.items
        .iter()
        .filter(|i| want.is_empty() || i.filter.to_lowercase().starts_with(&want))
        .take(200)
        .collect()
}

/// The text typed since the completion's word began, or `None` when the caret has left the word.
fn prefix_of(app: &App, c: &Completion) -> Option<String> {
    let tab = app.tabs.get(app.active).filter(|t| t.doc.id() == c.doc)?;
    let b = tab.doc.buffer();
    let head = b.primary().head;
    if head < c.anchor || b.selections().len() > 1 {
        return None;
    }
    let typed = b.slice(c.anchor..head);
    typed.chars().all(is_word).then_some(typed)
}

/// Keep the popup honest after each frame: close it when what it was about is gone, and offer
/// completion while a word is being typed.
pub fn update(app: &mut App, typed: bool) {
    match &mut app.popup {
        Popup::None => {}
        Popup::Hover { doc, state, .. } => {
            let now = app
                .tabs
                .get(app.active)
                .filter(|t| t.doc.id() == *doc)
                .map(|t| (t.doc.buffer().revision(), t.doc.buffer().primary().head));
            if now != Some(*state) {
                app.popup = Popup::None;
            }
        }
        Popup::Completion(_) => {
            let keep = match &app.popup {
                Popup::Completion(c) => prefix_of(app, c)
                    .map(|p| !visible(c, &p).is_empty())
                    .unwrap_or(false),
                _ => false,
            };
            if !keep {
                app.popup = Popup::None;
            } else if let Popup::Completion(c) = &mut app.popup {
                c.selected = c.selected.min(199);
            }
        }
    }
    if !typed || !app.settings.language_servers {
        return;
    }
    if matches!(app.popup, Popup::Completion(_)) {
        return;
    }
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let b = tab.doc.buffer();
    let head = b.primary().head;
    if head == 0 || b.selections().len() > 1 {
        return;
    }
    let before = b.slice(head - 1..head);
    let word_len = {
        let mut n = 0;
        while n < head && b.slice(head - 1 - n..head - n).chars().all(is_word) && n < 3 {
            n += 1;
        }
        n
    };
    if before == "." || word_len >= 2 {
        ask(app, QueryKind::Completion, true);
    }
}

/// Keys that belong to an open completion list. Call before the editor sees the keyboard.
pub fn keys(app: &mut App, ctx: &egui::Context) {
    let Popup::Completion(c) = &app.popup else {
        if matches!(app.popup, Popup::Hover { .. })
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            app.popup = Popup::None;
        }
        return;
    };
    let count = prefix_of(app, c).map_or(0, |p| visible(c, &p).len());
    let (down, up, accept, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if escape {
        app.popup = Popup::None;
        return;
    }
    if accept {
        accept_selected(app);
        return;
    }
    if let Popup::Completion(c) = &mut app.popup {
        if down && count > 0 {
            c.selected = (c.selected + 1) % count;
        }
        if up && count > 0 {
            c.selected = (c.selected + count - 1) % count;
        }
    }
}

fn accept_selected(app: &mut App) {
    let Popup::Completion(c) = std::mem::take(&mut app.popup) else {
        return;
    };
    let Some(prefix) = prefix_of(app, &c) else {
        return;
    };
    let Some(insert) = visible(&c, &prefix)
        .get(c.selected)
        .map(|i| i.insert.clone())
    else {
        return;
    };
    let Some(tab) = app.tabs.get_mut(app.active) else {
        return;
    };
    let head = tab.doc.buffer().primary().head;
    let end = c.anchor + insert.chars().count();
    tab.doc.apply(|b| {
        b.apply_edits(vec![Edit::new(c.anchor..head, insert)])
            .unwrap_or_default()
    });
    tab.doc.navigate(|b| b.set_cursor(end));
    tab.view.reveal = true;
}

fn kind_glyph(kind: u8) -> &'static str {
    match kind {
        2..=4 => "ƒ",
        5 | 6 | 10 => "◇",
        7 | 8 | 22 => "▢",
        9 | 19 | 17 | 18 => "▤",
        14 => "¶",
        _ => "·",
    }
}

/// Draw the popup, under the caret.
pub fn show(ctx: &egui::Context, app: &mut App, look: &Look) {
    let Some(caret) = app.tabs.get(app.active).and_then(|t| t.view.caret_rect) else {
        return;
    };
    let p = &look.palette;
    let frame = egui::Frame::NONE
        .fill(color(p.surface_panel))
        .stroke(Stroke::new(1.0, color(p.border_default)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 6));
    let at = pos2(caret.left(), caret.bottom() + 4.0);
    match &app.popup {
        Popup::None => {}
        Popup::Hover { text, .. } => {
            let clean: String = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("```"))
                .collect::<Vec<_>>()
                .join("\n");
            egui::Area::new(egui::Id::unique("codigo-hover"))
                .order(Order::Foreground)
                .fixed_pos(at)
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.set_max_width(560.0);
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(clean)
                                        .font(FontId::monospace(12.5))
                                        .color(color(p.text_primary)),
                                );
                            });
                    });
                });
        }
        Popup::Completion(c) => {
            let Some(prefix) = prefix_of(app, c) else {
                return;
            };
            let items = visible(c, &prefix);
            let selected = c.selected.min(items.len().saturating_sub(1));
            let first = selected.saturating_sub(ROWS - 1);
            egui::Area::new(egui::Id::unique("codigo-completion"))
                .order(Order::Foreground)
                .fixed_pos(at)
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.set_min_width(300.0);
                        ui.set_max_width(520.0);
                        for (n, item) in items.iter().enumerate().skip(first).take(ROWS) {
                            let (rect, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 22.0),
                                egui::Sense::hover(),
                            );
                            if n == selected {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(6),
                                    color(p.accent_default).gamma_multiply(0.25),
                                );
                            }
                            let ty = rect.center().y;
                            ui.painter().text(
                                pos2(rect.left() + 8.0, ty),
                                Align2::LEFT_CENTER,
                                kind_glyph(item.kind),
                                FontId::proportional(13.0),
                                color(p.accent_default),
                            );
                            ui.painter().text(
                                pos2(rect.left() + 28.0, ty),
                                Align2::LEFT_CENTER,
                                &item.label,
                                FontId::monospace(13.0),
                                color(p.text_primary),
                            );
                            if let Some(detail) = &item.detail {
                                let detail: String = detail.chars().take(40).collect();
                                ui.painter().text(
                                    pos2(rect.right() - 8.0, ty),
                                    Align2::RIGHT_CENTER,
                                    detail,
                                    FontId::proportional(11.5),
                                    color(p.text_tertiary),
                                );
                            }
                        }
                    });
                });
        }
    }
}
