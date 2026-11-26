// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a language server answers beyond diagnostics: go to definition, hover text and completion.
//! The request goes out on a command, the answer is polled each frame, and the result is either a
//! jump or a popup next to the caret.

use codigo_engine::{char_in_line, Answer, CompletionItem, DocId, Query, QueryError, QueryKind};
use codigo_text::Edit;
use egui::{pos2, vec2, Align2, CornerRadius, FontId, Key, Modifiers, Order, Stroke};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::tr;

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
            if !app.open_path(&place.path, Some(place.line), true) {
                return;
            }
            if let Some(tab) = app.tabs.get_mut(app.active) {
                let col = char_in_line(&tab.doc, place.line, place.character);
                tab.doc.navigate(|b| {
                    let line = place.line.min(b.line_count().saturating_sub(1));
                    b.set_cursor(b.line_start(line) + col);
                });
                tab.view.reveal = true;
            }
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
