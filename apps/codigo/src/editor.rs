// SPDX-License-Identifier: MIT OR Apache-2.0
//! The text editor widget: draws a document on a cell grid, turns the mouse and keyboard into
//! edits, and keeps the caret in view.
//!
//! Only the rows on screen are laid out, so a file of a million lines costs what a screenful
//! does. The one thing that is not lazy is word wrap, which needs every line's height up front;
//! past [`WRAP_LINE_LIMIT`] lines it is switched off rather than made slow.

use codigo_engine::{Cmd, Document, Mark, Severity};
use codigo_syntax::Span;
use codigo_text::search::Matcher;
use codigo_text::{Buffer, IndentRules, Motion, Pairs};
use codigo_workspace::{LineChange, LineChangeKind};
use std::cell::RefCell;
use std::rc::Rc;

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, Event, EventFilter, FontId, Id, Key, Modifiers,
    PointerButton, Rect, Sense, Stroke, StrokeKind, Ui,
};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

use crate::layout::{self, cells_of};
use codigo_engine::settings::CursorStyle;
use codigo_engine::theme::{Colors, Rgb, Slot};

/// Word wrap needs a row count for every line; beyond this many lines it is not offered.
pub const WRAP_LINE_LIMIT: usize = 50_000;
/// Space between the gutter and the first column, and after the last, in points.
const PAD: f32 = 10.0;

#[derive(Clone, Debug)]
pub struct Options {
    pub font_size: f32,
    /// Row height as a multiple of the font size.
    pub line_height: f32,
    /// The colours of the text, of the selection, of the current line and of each kind of code.
    pub colors: std::sync::Arc<Colors>,
    pub cursor: CursorStyle,
    pub cursor_blink: bool,
    /// Columns where a thin vertical line is drawn.
    pub rulers: Vec<u16>,
    pub indent_guides: bool,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub highlight_line: bool,
    pub whitespace: bool,
    pub auto_close_pairs: bool,
    pub minimap: bool,
    pub blame: bool,
}

/// Per-tab view state that is not part of the document.
#[derive(Default)]
pub struct View {
    cache: Cache,
    /// Scroll the caret into view on the next frame.
    pub reveal: bool,
    /// Lines the working copy changed against git, for the gutter.
    pub marks: Vec<LineChange>,
    /// Lines (zero-based) with a breakpoint, sorted, for the gutter.
    pub breakpoints: Vec<usize>,
    /// Who last changed each line, and the revision of the text that was read, for the blame
    /// shown after the caret's line.
    pub blame: Option<std::sync::Arc<(u64, Vec<codigo_git::BlameLine>)>>,
    /// The line the debugger is stopped on, if it is stopped in this file.
    pub exec_line: Option<usize>,
    /// Time of the last edit or caret move, to keep the caret solid while typing.
    last_input: f64,
    /// Rows a page is, from the last frame.
    page_rows: usize,
    /// The primary caret and revision the last frame ended with.
    last_state: (u64, usize),
    /// Where the caret was last drawn, for the status bar and popups.
    pub caret_rect: Option<Rect>,
    /// What the language server reported, sorted by line, for squiggles and tooltips.
    pub diagnostics: Vec<Mark>,
    /// The hub generation and document revision `diagnostics` was built for.
    pub diagnostics_key: Option<(u64, u64)>,
    /// 0 for the editor's own view of a file, 1 for the second view of the same file in a split.
    pub pane: u8,
    /// How far the text is scrolled, in points, as of the last frame.
    scroll_y: f32,
    /// Scroll here on the next frame (the minimap was clicked).
    set_scroll: Option<f32>,
    /// Row height and the height of the text area, from the last frame, for the minimap.
    row_h: f32,
    viewport_h: f32,
}

impl View {
    pub fn new() -> Self {
        View {
            reveal: true,
            page_rows: 20,
            ..View::default()
        }
    }
}

#[derive(Default)]
struct Cache {
    revision: u64,
    tab: usize,
    wrap_cols: usize,
    valid: bool,
    /// `first_row[line]` is the visual row a line starts at; one more entry for the end. Empty
    /// without word wrap.
    first_row: Vec<u32>,
    max_cols: usize,
    /// The last very long line that was laid out, which would otherwise be laid out again every
    /// frame (a minified bundle is one line of megabytes).
    long: RefCell<Option<LongLine>>,
    /// Progress of the scan for the widest line: the text it is for, how many lines are done,
    /// and the widest so far.
    scan_revision: Option<(u64, usize)>,
    scanned: usize,
    scan_max: usize,
}

/// How long one frame may spend looking for the widest line of a big file.
const SCAN_BUDGET: std::time::Duration = std::time::Duration::from_millis(5);

struct LongLine {
    revision: u64,
    line: usize,
    wrap_cols: usize,
    tab: usize,
    laid: Rc<Laid>,
}

/// Lines with more characters than this are kept laid out between frames.
const LONG_LINE_CHARS: usize = 2_000;

/// What the editor asks the app to do.
#[derive(Default)]
pub struct Output {
    pub commands: Vec<Cmd>,
    /// The person clicked in the editor.
    pub clicked: bool,
    /// The document's text changed this frame.
    pub edited: bool,
    /// Text for the system clipboard.
    pub copied: Option<String>,
    /// The gutter was clicked on this line (zero-based): toggle a breakpoint.
    pub gutter_click: Option<usize>,
}

pub fn indent_rules(language_id: &str) -> IndentRules {
    match language_id {
        "python" | "yaml" | "nim" | "haskell" => IndentRules {
            open: "{[(",
            colon: true,
        },
        _ => IndentRules::default(),
    }
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

struct Metrics {
    cw: f32,
    row_h: f32,
    font: FontId,
    gutter: f32,
    cols: usize,
}

/// Work out the row layout. Without word wrap this only finds the widest line, which on a huge
/// file is spread over several frames; the result says whether it is finished.
fn cache_for(doc: &Document, cache: &mut Cache, cols: usize, wrap: bool) -> bool {
    let buffer = doc.buffer();
    let tab = buffer.tab_width.max(1);
    let wrap_cols = if wrap { cols } else { 0 };
    if cache.valid
        && cache.revision == buffer.revision()
        && cache.tab == tab
        && cache.wrap_cols == wrap_cols
    {
        return true;
    }
    let rope = buffer.rope();
    let lines = rope.len_lines();
    if !wrap {
        // Start over when the text or the tab width changed; the old width stays as a lower
        // bound meanwhile so the scroll area does not jump.
        if cache.scan_revision != Some((buffer.revision(), tab)) {
            cache.scan_revision = Some((buffer.revision(), tab));
            cache.scanned = 0;
            cache.scan_max = 0;
        }
        let started = std::time::Instant::now();
        let mut line = cache.scanned;
        let mut max_cols = cache.scan_max;
        for text in rope.lines_at(line) {
            let chars = text.len_chars();
            // Cheap bound first: a line can be at most `max(2, tab)` cells per character.
            if chars.saturating_mul(tab.max(2)) > max_cols {
                let text = text.to_string();
                max_cols = max_cols.max(layout::width_of(text.trim_end_matches(['\n', '\r']), tab));
            }
            line += 1;
            if line.is_multiple_of(2048) && started.elapsed() > SCAN_BUDGET {
                break;
            }
        }
        cache.scanned = line;
        cache.scan_max = max_cols;
        cache.max_cols = cache.max_cols.max(max_cols);
        cache.wrap_cols = 0;
        cache.first_row = Vec::new();
        if line < lines {
            return false;
        }
        cache.max_cols = max_cols;
        cache.revision = buffer.revision();
        cache.tab = tab;
        cache.valid = true;
        return true;
    }
    let mut max_cols = 0usize;
    let mut first_row = Vec::with_capacity(lines + 1);
    let mut rows = 0u32;
    for l in 0..lines {
        let line = rope.line(l);
        let text = line.to_string();
        let width = layout::width_of(text.trim_end_matches(['\n', '\r']), tab);
        max_cols = max_cols.max(width);
        first_row.push(rows);
        let count = if width <= cols.max(2) {
            1
        } else {
            let text = text.trim_end_matches(['\n', '\r']);
            let cells = cells_of(text, tab);
            layout::wrap(text, &cells, cols).len() as u32
        };
        rows = rows.saturating_add(count);
    }
    first_row.push(rows);
    cache.revision = buffer.revision();
    cache.tab = tab;
    cache.wrap_cols = wrap_cols;
    cache.valid = true;
    cache.first_row = first_row;
    cache.max_cols = max_cols;
    true
}

fn total_rows(cache: &Cache, lines: usize) -> usize {
    cache
        .first_row
        .last()
        .map_or(lines, |last| *last as usize)
        .max(1)
}

fn line_of_row(cache: &Cache, lines: usize, row: usize) -> usize {
    if cache.first_row.is_empty() {
        return row.min(lines.saturating_sub(1));
    }
    let at = cache
        .first_row
        .partition_point(|first| (*first as usize) <= row);
    at.saturating_sub(1).min(lines.saturating_sub(1))
}

fn first_row_of(cache: &Cache, line: usize) -> usize {
    if cache.first_row.is_empty() {
        line
    } else {
        cache.first_row.get(line).map_or(0, |r| *r as usize)
    }
}

/// One line prepared for drawing and hit testing.
struct Laid {
    text: String,
    cells: Vec<u32>,
    rows: Vec<(usize, usize)>,
    /// The byte offset of each character, and one more for the end.
    bytes: Vec<usize>,
}

fn lay(buffer: &Buffer, line: usize, wrap_cols: usize) -> Laid {
    let text = buffer.line_text(line);
    let cells = cells_of(&text, buffer.tab_width.max(1));
    let rows = if wrap_cols > 0 {
        layout::wrap(&text, &cells, wrap_cols)
    } else {
        vec![(0, cells.len().saturating_sub(1))]
    };
    let bytes = byte_of_chars(&text);
    Laid {
        text,
        cells,
        rows,
        bytes,
    }
}

/// [`lay`], remembering the result for lines long enough that doing it every frame hurts.
fn lay_cached(cache: &Cache, buffer: &Buffer, line: usize, wrap_cols: usize) -> Rc<Laid> {
    let chars = buffer.rope().get_line(line).map_or(0, |l| l.len_chars());
    if chars < LONG_LINE_CHARS {
        return Rc::new(lay(buffer, line, wrap_cols));
    }
    let (revision, tab) = (buffer.revision(), buffer.tab_width.max(1));
    if let Some(l) = &*cache.long.borrow() {
        if l.revision == revision && l.line == line && l.wrap_cols == wrap_cols && l.tab == tab {
            return l.laid.clone();
        }
    }
    let laid = Rc::new(lay(buffer, line, wrap_cols));
    *cache.long.borrow_mut() = Some(LongLine {
        revision,
        line,
        wrap_cols,
        tab,
        laid: laid.clone(),
    });
    laid
}

/// The row of a line that holds the character `col`.
fn row_holding(rows: &[(usize, usize)], col: usize) -> usize {
    for (i, (_, end)) in rows.iter().enumerate() {
        if col < *end || i + 1 == rows.len() {
            return i;
        }
    }
    0
}

fn byte_of_chars(text: &str) -> Vec<usize> {
    let mut out: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    out.push(text.len());
    out
}

/// The id of the text widget showing `doc` in `pane`, to ask whether it has the keyboard.
pub fn editor_id(doc: u64, pane: u8) -> Id {
    Id::unique(("codigo-editor", doc, pane))
}

/// The editor, with the minimap beside it when it is on.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    doc: &mut Document,
    view: &mut View,
    opts: &Options,
    look: &Look,
    matcher: Option<&Matcher>,
    focus: bool,
    now: f64,
) -> Output {
    let full = ui.available_rect_before_wrap();
    if !opts.minimap || full.width() < MINIMAP_MIN_AREA {
        return show_text(ui, doc, view, opts, look, matcher, focus, now);
    }
    let strip = Rect::from_min_max(pos2(full.right() - MINIMAP_W, full.top()), full.max);
    let text = Rect::from_min_max(full.min, pos2(strip.left(), full.bottom()));
    let out = ui
        .scope_builder(egui::UiBuilder::new().max_rect(text), |ui| {
            show_text(ui, doc, view, opts, look, matcher, focus, now)
        })
        .inner;
    minimap(ui, doc, view, strip, look);
    out
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// Width of the minimap strip, and how wide the editor must be to afford one.
const MINIMAP_W: f32 = 92.0;
const MINIMAP_MIN_AREA: f32 = 480.0;

/// A scaled-down picture of the whole file: one thin bar per run of text, the part on screen
/// lit, and ticks for problems and changes. Click or drag to scroll.
fn minimap(ui: &mut Ui, doc: &Document, view: &mut View, strip: Rect, look: &Look) {
    let p = look.palette;
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, 0.0, color(p.surface_window));
    painter.line_segment(
        [strip.left_top(), strip.left_bottom()],
        Stroke::new(1.0, color(p.border_subtle)),
    );
    let buffer = doc.buffer();
    let lines = buffer.line_count().max(1);
    let h = strip.height();
    if h < 8.0 || view.row_h <= 0.0 {
        return;
    }
    // Points per line: two when the file is short enough, less when it has to fit.
    let scale = (h / lines as f32).min(2.0);
    let step = scale.max(1.0);
    let bars = ((lines as f32 * scale) / step).ceil() as usize;
    let ink = color(p.text_secondary).gamma_multiply(0.45);
    let x0 = strip.left() + 6.0;
    let col_w = 0.8_f32;
    let max_cols = ((strip.width() - 18.0) / col_w) as usize;
    let thick = (step - 0.4).clamp(1.0, 1.6);
    let rope = buffer.rope();
    for i in 0..bars {
        let line = ((i as f32 * step) / scale) as usize;
        let Some(text) = rope.get_line(line) else {
            break;
        };
        let y = strip.top() + i as f32 * step;
        if y > strip.bottom() {
            break;
        }
        let (mut col, mut run_start): (usize, Option<usize>) = (0, None);
        for ch in text.chars().take(max_cols * 2) {
            if col >= max_cols {
                break;
            }
            let blank = ch.is_whitespace();
            match (blank, run_start) {
                (false, None) => run_start = Some(col),
                (true, Some(a)) => {
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(x0 + a as f32 * col_w, y),
                            vec2((col - a) as f32 * col_w, thick),
                        ),
                        0.0,
                        ink,
                    );
                    run_start = None;
                }
                _ => {}
            }
            col += if ch == '\t' { 4 } else { 1 };
        }
        if let Some(a) = run_start {
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(x0 + a as f32 * col_w, y),
                    vec2((col.min(max_cols) - a) as f32 * col_w, thick),
                ),
                0.0,
                ink,
            );
        }
    }
    let y_of = |line: usize| strip.top() + line as f32 * scale;
    // Where the file differs from git, and what the language server complains about.
    for change in &view.marks {
        let fg = match change.kind {
            LineChangeKind::Added => p.status_success_fg,
            LineChangeKind::Modified => p.status_info_fg,
            LineChangeKind::Removed => p.status_danger_fg,
        };
        let top = y_of(change.line);
        let bottom = y_of(change.line + change.count.max(1)).max(top + 2.0);
        painter.rect_filled(
            Rect::from_min_max(
                pos2(strip.left() + 1.0, top),
                pos2(strip.left() + 4.0, bottom),
            ),
            0.0,
            color(fg),
        );
    }
    for d in &view.diagnostics {
        if d.severity == Severity::Hint {
            continue;
        }
        let top = y_of(d.line);
        painter.rect_filled(
            Rect::from_min_max(
                pos2(strip.right() - 5.0, top),
                pos2(strip.right() - 1.0, top + 2.5),
            ),
            0.0,
            severity_ink(look, d.severity),
        );
    }
    // The part of the file on screen.
    let rows = total_rows(&view.cache, lines);
    let first_row = (view.scroll_y / view.row_h).max(0.0) as usize;
    let last_row = (((view.scroll_y + view.viewport_h) / view.row_h).ceil() as usize).min(rows);
    let top_line = line_of_row(&view.cache, lines, first_row);
    let bottom_line = line_of_row(&view.cache, lines, last_row).max(top_line + 1);
    let lit = Rect::from_min_max(
        pos2(strip.left() + 1.0, y_of(top_line)),
        pos2(strip.right(), y_of(bottom_line).min(strip.bottom())),
    );
    painter.rect_filled(lit, 0.0, color(p.text_primary).gamma_multiply(0.08));
    painter.rect_stroke(
        lit,
        0.0,
        Stroke::new(1.0, color(p.text_primary).gamma_multiply(0.18)),
        egui::StrokeKind::Inside,
    );
    // Click or drag to scroll: the point under the pointer becomes the middle of the screen.
    let response = ui.interact(
        strip,
        Id::unique(("codigo-minimap", doc.id().0, view.pane)),
        Sense::click_and_drag(),
    );
    if response.clicked() || response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let line = (((pos.y - strip.top()) / scale).max(0.0) as usize).min(lines - 1);
            let row = first_row_of(&view.cache, line) as f32;
            view.set_scroll = Some((row * view.row_h - view.viewport_h / 2.0).max(0.0));
            ui.ctx().request_repaint();
        }
    }
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

#[allow(clippy::too_many_arguments)]
fn show_text(
    ui: &mut Ui,
    doc: &mut Document,
    view: &mut View,
    opts: &Options,
    look: &Look,
    matcher: Option<&Matcher>,
    focus: bool,
    now: f64,
) -> Output {
    let mut out = Output::default();
    let id = editor_id(doc.id().0, view.pane);
    let p = look.palette;
    if focus {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }

    // Metrics.
    let font = FontId::monospace(opts.font_size);
    let cw = ui
        .painter()
        .layout_no_wrap("M".into(), font.clone(), Color32::WHITE)
        .size()
        .x
        .max(1.0);
    let row_h = (opts.font_size * opts.line_height).round().max(10.0);
    let lines = doc.buffer().line_count();
    let gutter = if opts.line_numbers {
        (digits(lines).max(3) as f32) * cw + PAD * 2.0 + 4.0
    } else {
        PAD
    };
    let avail = ui.available_width();
    let cols = (((avail - gutter - PAD - 16.0) / cw).floor().max(8.0)) as usize;
    let wrap = opts.word_wrap && lines <= WRAP_LINE_LIMIT;
    let m = Metrics {
        cw,
        row_h,
        font,
        gutter,
        cols,
    };

    // Keyboard first, so what is drawn is the state after it.
    let before = (doc.buffer().revision(), doc.buffer().primary().head);
    if focused {
        handle_keys(ui, doc, view, opts, &mut out);
    }

    if doc.ensure_parsed() {
        // The colours of a big file are still being worked out: draw again as soon as they are.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    if !cache_for(doc, &mut view.cache, m.cols, wrap) {
        ui.ctx().request_repaint();
    }
    let wrap_cols = if wrap { m.cols } else { 0 };
    let rows_total = total_rows(&view.cache, doc.buffer().line_count());

    let content_w = if wrap {
        avail
    } else {
        m.gutter + PAD + (view.cache.max_cols as f32 + 6.0) * m.cw
    };
    let area = if wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    let outer = ui.available_rect_before_wrap();
    let viewport_h = ui.available_height();
    let content_h = rows_total as f32 * m.row_h + (viewport_h - m.row_h * 3.0).max(0.0);
    let reveal = std::mem::take(&mut view.reveal);
    view.row_h = m.row_h;
    view.viewport_h = viewport_h;
    let area = match view.set_scroll.take() {
        Some(y) => area.vertical_scroll_offset(y),
        None => area,
    };

    let scrolled = area
        .id_salt(("codigo-scroll", doc.id().0, view.pane))
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show_viewport(ui, |ui, viewport| {
            let size = vec2(
                content_w.max(viewport.width()),
                content_h.max(viewport.height()),
            );
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let response = ui.interact(rect, id, Sense::click_and_drag());
            let origin = rect.min;
            // The scroll area only clips along the axes that scroll, so bound it by its own frame.
            let clip = ui.clip_rect().intersect(outer);
            let text_x = origin.x + m.gutter;
            view.page_rows = ((viewport.height() / m.row_h) as usize)
                .saturating_sub(1)
                .max(1);

            // The pointer.
            pointer(
                ui, doc, view, &response, &m, text_x, origin.y, wrap_cols, opts, &mut out,
            );

            diagnostic_tooltip(doc, view, &response, &m, text_x, origin.y, wrap_cols, look);

            // Which rows are on screen.
            let lines = doc.buffer().line_count();
            let first_row = (viewport.min.y / m.row_h).floor().max(0.0) as usize;
            let last_row = (viewport.max.y / m.row_h).ceil().max(0.0) as usize + 1;
            let first_line = line_of_row(&view.cache, lines, first_row);
            let mut last_line = line_of_row(&view.cache, lines, last_row);
            last_line = (last_line + 1).min(lines);

            let spans: Vec<Vec<Span>> = {
                let rope = doc.buffer().rope().clone();
                match doc.syntax_mut() {
                    Some(s) => s.spans(&rope, first_line..last_line),
                    None => Vec::new(),
                }
            };

            let painter = ui.painter_at(clip);
            // The text area paints its own surface, rounded like the island at its bottom edge.
            let r = nodalia_theme::engineering::layout::ISLAND_RADIUS as u8;
            painter.rect_filled(
                clip,
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: r,
                    se: r,
                },
                color(p.surface_panel),
            );
            let selections: Vec<(usize, usize, usize)> = doc
                .buffer()
                .selections()
                .iter()
                .map(|s| (s.start(), s.end(), s.head))
                .collect();
            let buffer = doc.buffer();
            let caret_line = buffer.line_of(buffer.primary().head);
            let selection_fill = color(p.selection_bg);
            let active_fill = c32(opts.colors.get(Slot::ActiveLine));
            let blink_on = {
                let since = now - view.last_input;
                !opts.cursor_blink || since < 0.6 || ((now * 1.8) as i64) % 2 == 0
            };
            let tab = buffer.tab_width.max(1);
            let col_lo = ((viewport.min.x - m.gutter) / m.cw).floor() as i64 - 2;
            let col_hi = ((viewport.max.x - m.gutter) / m.cw).ceil() as i64 + 2;
            let mut caret_rect: Option<Rect> = None;
            for column in &opts.rulers {
                let x = text_x + f32::from(*column) * m.cw;
                if x > clip.left() + m.gutter && x < clip.right() {
                    painter.line_segment(
                        [pos2(x, clip.top()), pos2(x, clip.bottom())],
                        Stroke::new(1.0, color(p.text_tertiary).gamma_multiply(0.3)),
                    );
                }
            }
            let mut gutter_items: Vec<(f32, usize, bool)> = Vec::new();

            for line in first_line..last_line {
                let laid = lay_cached(&view.cache, buffer, line, wrap_cols);
                let line_start = buffer.line_start(line);
                let chars = laid.cells.len().saturating_sub(1);
                let bytes = &laid.bytes;
                let line_spans: &[Span] = spans
                    .get(line - first_line)
                    .map_or(&[][..], |s| s.as_slice());
                let find_hits: Vec<(usize, usize)> = matcher
                    .map(|mt| {
                        mt.in_text(&laid.text)
                            .into_iter()
                            .map(|(a, b)| {
                                (
                                    bytes.partition_point(|x| *x < a),
                                    bytes.partition_point(|x| *x < b),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let base_row = first_row_of(&view.cache, line);
                for (ri, (a, b)) in laid.rows.iter().copied().enumerate() {
                    let y = origin.y + (base_row + ri) as f32 * m.row_h;
                    if y + m.row_h < clip.top() || y > clip.bottom() {
                        continue;
                    }
                    let base_cell = laid.cells.get(a).copied().unwrap_or(0);
                    let x_of = |idx: usize| -> f32 {
                        let cell = laid.cells.get(idx).copied().unwrap_or(base_cell);
                        text_x + (cell.saturating_sub(base_cell)) as f32 * m.cw
                    };
                    let row_rect = Rect::from_min_size(
                        pos2(clip.left() + m.gutter, y),
                        vec2(clip.width() - m.gutter, m.row_h),
                    );
                    if opts.indent_guides {
                        let lead = laid
                            .text
                            .chars()
                            .take_while(|c| *c == ' ' || *c == '\t')
                            .count();
                        let indent = laid.cells.get(lead).copied().unwrap_or(0) as usize;
                        let guide = Stroke::new(1.0, color(p.text_tertiary).gamma_multiply(0.22));
                        let mut level = 0;
                        while level < indent {
                            let x = text_x + level as f32 * m.cw;
                            painter.line_segment([pos2(x, y), pos2(x, y + m.row_h)], guide);
                            level += tab;
                        }
                    }
                    let selections_here = selections.iter().any(|s| s.0 != s.1);
                    if opts.highlight_line
                        && line == caret_line
                        && !selections_here
                        && ri == row_holding(&laid.rows, buffer.primary().head - line_start)
                    {
                        painter.rect_filled(row_rect, 0.0, active_fill);
                    }
                    if ri == 0 && view.exec_line == Some(line) {
                        painter.rect_filled(
                            Rect::from_min_max(
                                pos2(clip.left() + m.gutter, y),
                                pos2(clip.right(), y + m.row_h),
                            ),
                            0.0,
                            color(p.status_warning_bg).gamma_multiply(0.7),
                        );
                    }
                    // Find matches.
                    for (ma, mb) in &find_hits {
                        let (sa, sb) = ((*ma).max(a), (*mb).min(b));
                        if sa < sb {
                            let r = Rect::from_min_max(
                                pos2(x_of(sa), y + 1.0),
                                pos2(x_of(sb), y + m.row_h - 1.0),
                            );
                            let current = selections
                                .iter()
                                .any(|s| s.0 == line_start + ma && s.1 == line_start + mb);
                            painter.rect_filled(
                                r,
                                CornerRadius::same(3),
                                if current {
                                    color(p.status_warning_bg)
                                } else {
                                    color(p.status_warning_bg).gamma_multiply(0.55)
                                },
                            );
                            if current {
                                painter.rect_stroke(
                                    r,
                                    CornerRadius::same(3),
                                    Stroke::new(1.0, color(p.status_warning_fg)),
                                    StrokeKind::Inside,
                                );
                            }
                        }
                    }
                    // Selections.
                    for (s0, s1, _) in &selections {
                        if s0 == s1 {
                            continue;
                        }
                        let (c0, c1) =
                            (s0.saturating_sub(line_start), s1.saturating_sub(line_start));
                        if *s1 < line_start || *s0 > line_start + chars {
                            continue;
                        }
                        let (sa, sb) = (c0.max(a), c1.min(b));
                        let last_row_of_line = ri + 1 == laid.rows.len();
                        let continues = *s1 > line_start + chars && last_row_of_line;
                        if sa < sb || (continues && sa <= b && c0 <= b) {
                            let xa = x_of(sa.min(b));
                            let mut xb = x_of(sb.max(sa).min(b));
                            if continues {
                                xb += m.cw * 0.6;
                            }
                            painter.rect_filled(
                                Rect::from_min_max(pos2(xa, y), pos2(xb, y + m.row_h)),
                                0.0,
                                selection_fill,
                            );
                        }
                    }
                    // Text.
                    paint_row(
                        &painter,
                        look,
                        &m,
                        opts,
                        &laid,
                        bytes,
                        line_spans,
                        a,
                        b,
                        tab,
                        (col_lo, col_hi),
                        text_x,
                        y,
                        base_cell,
                    );
                    // Blame, on the caret's line only, after its text.
                    if opts.blame && line == caret_line && ri + 1 == laid.rows.len() {
                        let entry = view
                            .blame
                            .as_ref()
                            .filter(|b| b.0 == buffer.revision())
                            .and_then(|b| b.1.get(line));
                        if let Some(entry) = entry {
                            painter.text(
                                pos2(x_of(b).max(text_x) + m.cw * 4.0, y + m.row_h / 2.0),
                                Align2::LEFT_CENTER,
                                crate::blame_ui::label(entry, crate::blame_ui::now_secs()),
                                m.font.clone(),
                                color(p.text_tertiary).gamma_multiply(0.85),
                            );
                        }
                    }
                    // Diagnostics.
                    let first = view.diagnostics.partition_point(|d| d.line < line);
                    for d in view
                        .diagnostics
                        .get(first..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|d| d.line == line)
                    {
                        let (sa, sb) = (d.start.max(a), d.end.min(b));
                        if sa < sb {
                            squiggle(
                                &painter,
                                x_of(sa),
                                x_of(sb),
                                y + m.row_h - 3.0,
                                severity_ink(look, d.severity),
                            );
                        }
                    }
                    // Carets.
                    for (_, _, head) in &selections {
                        if *head < line_start || *head > line_start + chars {
                            continue;
                        }
                        let c = head - line_start;
                        if row_holding(&laid.rows, c) != ri {
                            continue;
                        }
                        let x = x_of(c);
                        let r = match opts.cursor {
                            CursorStyle::Line => {
                                Rect::from_min_size(pos2(x, y + 1.0), vec2(2.0, m.row_h - 2.0))
                            }
                            CursorStyle::Block => {
                                Rect::from_min_size(pos2(x, y + 1.0), vec2(m.cw, m.row_h - 2.0))
                            }
                            CursorStyle::Underline => {
                                Rect::from_min_size(pos2(x, y + m.row_h - 3.0), vec2(m.cw, 2.0))
                            }
                        };
                        if *head == buffer.primary().head {
                            caret_rect = Some(r);
                        }
                        if focused && blink_on {
                            let fill = color(p.accent_default);
                            let fill = if opts.cursor == CursorStyle::Block {
                                fill.gamma_multiply(0.55)
                            } else {
                                fill
                            };
                            painter.rect_filled(r, 1.0, fill);
                        } else if !focused {
                            painter.rect_stroke(
                                r.expand2(vec2(2.0, 0.0)),
                                0.0,
                                Stroke::new(1.0, color(p.text_tertiary)),
                                StrokeKind::Inside,
                            );
                        }
                    }
                    if ri == 0 {
                        gutter_items.push((y, line, line == caret_line));
                    }
                }
            }

            // The gutter stays at the left edge whatever the horizontal scroll.
            let gx = clip.left();
            let gutter_rect = Rect::from_min_max(
                pos2(gx, clip.top()),
                pos2(gx + m.gutter - PAD * 0.4, clip.bottom()),
            );
            painter.rect_filled(gutter_rect, 0.0, color(p.surface_panel));
            if opts.line_numbers {
                for (y, line, active) in &gutter_items {
                    painter.text(
                        pos2(gx + m.gutter - PAD - 4.0, y + m.row_h / 2.0),
                        Align2::RIGHT_CENTER,
                        (line + 1).to_string(),
                        m.font.clone(),
                        if *active {
                            color(p.text_primary)
                        } else {
                            color(p.text_tertiary)
                        },
                    );
                }
            }
            for (y, line, _) in &gutter_items {
                let centre = pos2(gx + 9.0, y + m.row_h / 2.0);
                if view.breakpoints.binary_search(line).is_ok() {
                    painter.circle_filled(centre, 4.5, color(p.status_danger_fg));
                }
                if view.exec_line == Some(*line) {
                    let tri = vec![
                        pos2(centre.x - 4.0, centre.y - 5.0),
                        pos2(centre.x + 5.0, centre.y),
                        pos2(centre.x - 4.0, centre.y + 5.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        tri,
                        color(p.status_warning_fg),
                        Stroke::NONE,
                    ));
                }
            }
            for change in &view.marks {
                let (fg, tall) = match change.kind {
                    LineChangeKind::Added => (p.status_success_fg, true),
                    LineChangeKind::Modified => (p.status_info_fg, true),
                    LineChangeKind::Removed => (p.status_danger_fg, false),
                };
                let top = origin.y + first_row_of(&view.cache, change.line) as f32 * m.row_h;
                let rect = if tall {
                    let last = first_row_of(&view.cache, change.line + change.count.max(1));
                    let bottom = origin.y + last as f32 * m.row_h;
                    Rect::from_min_max(
                        pos2(gx + m.gutter - PAD * 0.4 - 3.0, top),
                        pos2(gx + m.gutter - PAD * 0.4, bottom),
                    )
                } else {
                    Rect::from_min_size(
                        pos2(gx + m.gutter - PAD * 0.4 - 6.0, top - 1.0),
                        vec2(6.0, 2.0),
                    )
                };
                painter.rect_filled(rect, 1.0, color(fg));
            }

            // A caret that jumped off screen (end of file, go to line) was not painted, so work
            // out where it would be.
            if reveal && caret_rect.is_none() {
                let buffer = doc.buffer();
                let head = buffer.primary().head;
                let line = buffer.line_of(head);
                let laid = lay_cached(&view.cache, buffer, line, wrap_cols);
                let c = head.saturating_sub(buffer.line_start(line));
                let row = first_row_of(&view.cache, line) + row_holding(&laid.rows, c);
                let y = origin.y + row as f32 * m.row_h;
                let x = origin.x + m.gutter + PAD;
                caret_rect = Some(Rect::from_min_size(
                    pos2(x, y + 1.0),
                    vec2(2.0, m.row_h - 2.0),
                ));
            }
            view.caret_rect = caret_rect;
            if reveal {
                if let Some(r) = caret_rect {
                    let want = r.expand2(vec2(m.cw * 3.0 + m.gutter, m.row_h * 1.5));
                    ui.scroll_to_rect(want, None);
                }
            }
            if focused {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(550));
            }
        });
    view.scroll_y = scrolled.state.clamped_offset().y;

    let after = (doc.buffer().revision(), doc.buffer().primary().head);
    if after != before || after != view.last_state {
        view.last_input = now;
        view.reveal = true;
        if after.0 != before.0 {
            out.edited = true;
        }
    }
    view.last_state = after;
    out
}

/// What the language server said about the text under the pointer.
#[allow(clippy::too_many_arguments)]
fn diagnostic_tooltip(
    doc: &Document,
    view: &View,
    response: &egui::Response,
    m: &Metrics,
    text_x: f32,
    origin_y: f32,
    wrap_cols: usize,
    look: &Look,
) {
    if view.diagnostics.is_empty() || !response.hovered() {
        return;
    }
    let Some(pos) = response.hover_pos().filter(|p| p.x >= text_x) else {
        return;
    };
    let idx = char_at_point(doc, view, m, text_x, origin_y, wrap_cols, pos);
    let (line, col) = doc.buffer().line_col(idx);
    let hits: Vec<&Mark> = view
        .diagnostics
        .iter()
        .filter(|d| d.line == line && col >= d.start && col < d.end)
        .take(5)
        .collect();
    if hits.is_empty() {
        return;
    }
    response.clone().on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(460.0);
        for d in hits {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(severity_ink(look, d.severity), "●");
                ui.label(&d.message);
            });
            if let Some(source) = &d.source {
                ui.label(
                    egui::RichText::new(source)
                        .small()
                        .color(color(look.palette.text_tertiary)),
                );
            }
        }
    });
}

fn severity_ink(look: &Look, severity: Severity) -> Color32 {
    let p = &look.palette;
    match severity {
        Severity::Error => color(p.status_danger_fg),
        Severity::Warning => color(p.status_warning_fg),
        Severity::Information => color(p.status_info_fg),
        Severity::Hint => color(p.text_tertiary),
    }
}

/// A wavy underline from `x0` to `x1` at height `y`.
fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, ink: Color32) {
    const STEP: f32 = 3.0;
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x < x1 && points.len() < 4_000 {
        points.push(pos2(x, if up { y - 1.5 } else { y }));
        up = !up;
        x += STEP;
    }
    points.push(pos2(x1, if up { y - 1.5 } else { y }));
    painter.add(egui::Shape::line(points, Stroke::new(1.2, ink)));
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    painter: &egui::Painter,
    look: &Look,
    m: &Metrics,
    opts: &Options,
    laid: &Laid,
    bytes: &[usize],
    spans: &[Span],
    a: usize,
    b: usize,
    tab: usize,
    (col_lo, col_hi): (i64, i64),
    text_x: f32,
    y: f32,
    base_cell: u32,
) {
    let p = &look.palette;
    let plain = color(p.text_primary);
    let ws_ink = color(p.text_tertiary).gamma_multiply(0.45);
    let ty = y + m.row_h / 2.0;
    let mut sp = 0usize;
    let mut run = String::new();
    let mut run_x = 0.0f32;
    let mut run_ink = plain;
    let flush = |run: &mut String, run_x: f32, run_ink: Color32| {
        if !run.is_empty() {
            painter.text(
                pos2(run_x, ty),
                Align2::LEFT_CENTER,
                std::mem::take(run),
                m.font.clone(),
                run_ink,
            );
        }
    };
    // Only the characters near the screen: a minified line is megabytes long.
    let row_cells = laid
        .cells
        .get(a..b.min(laid.cells.len()))
        .unwrap_or_default();
    let first = a + row_cells
        .partition_point(|c| i64::from(*c) < i64::from(base_cell) + col_lo)
        .saturating_sub(1);
    let last =
        (a + row_cells.partition_point(|c| i64::from(*c) <= i64::from(base_cell) + col_hi) + 1)
            .min(b);
    let window = match (bytes.get(first), bytes.get(last)) {
        (Some(from), Some(to)) => laid.text.get(*from..*to).unwrap_or_default(),
        _ => "",
    };
    for (k, c) in window.chars().enumerate() {
        let i = first + k;
        let cell = laid
            .cells
            .get(i)
            .copied()
            .unwrap_or(0)
            .saturating_sub(base_cell);
        let next = laid.cells.get(i + 1).copied().unwrap_or(cell + base_cell);
        let (cell, next) = (i64::from(cell), i64::from(next.saturating_sub(base_cell)));
        if next < col_lo || cell > col_hi {
            flush(&mut run, run_x, run_ink);
            continue;
        }
        let x = text_x + cell as f32 * m.cw;
        // Scope of this character: spans are sorted and do not overlap.
        let byte = bytes.get(i).copied().unwrap_or(0);
        while spans.get(sp).is_some_and(|s| s.end <= byte) {
            sp += 1;
        }
        let ink = match spans.get(sp) {
            Some(s) if s.start <= byte => c32(opts.colors.scope(s.scope)),
            _ => plain,
        };
        match c {
            '\t' => {
                flush(&mut run, run_x, run_ink);
                if opts.whitespace {
                    painter.text(
                        pos2(x, ty),
                        Align2::LEFT_CENTER,
                        "→",
                        m.font.clone(),
                        ws_ink,
                    );
                }
            }
            ' ' => {
                flush(&mut run, run_x, run_ink);
                if opts.whitespace {
                    painter.text(
                        pos2(x + m.cw * 0.5, ty),
                        Align2::CENTER_CENTER,
                        "·",
                        m.font.clone(),
                        ws_ink,
                    );
                }
            }
            c if c.is_ascii() && !c.is_control() => {
                if !run.is_empty() && (ink != run_ink) {
                    flush(&mut run, run_x, run_ink);
                }
                if run.is_empty() {
                    run_x = x;
                    run_ink = ink;
                }
                run.push(c);
            }
            c if c.is_control() => {}
            c => {
                flush(&mut run, run_x, run_ink);
                if next > cell {
                    painter.text(pos2(x, ty), Align2::LEFT_CENTER, c, m.font.clone(), ink);
                }
            }
        }
        // A space that is not part of a run breaks it; keep runs contiguous in the grid.
        let _ = tab;
    }
    flush(&mut run, run_x, run_ink);
}

#[allow(clippy::too_many_arguments)]
fn pointer(
    ui: &mut Ui,
    doc: &mut Document,
    view: &mut View,
    response: &egui::Response,
    m: &Metrics,
    text_x: f32,
    origin_y: f32,
    wrap_cols: usize,
    _opts: &Options,
    out: &mut Output,
) {
    let (pos, pressed, mods) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.button_pressed(PointerButton::Primary),
            i.modifiers,
        )
    });
    let Some(pos) = pos else { return };
    let at = |doc: &Document, view: &View| -> usize {
        char_at_point(doc, view, m, text_x, origin_y, wrap_cols, pos)
    };
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if pressed && response.hovered() && pos.x < text_x {
        let idx = at(doc, view);
        out.gutter_click = Some(doc.buffer().line_of(idx));
        return;
    }
    if pressed && response.hovered() {
        response.request_focus();
        out.clicked = true;
        let idx = at(doc, view);
        if mods.alt {
            doc.navigate(|b| b.add_cursor(idx));
        } else if mods.shift {
            doc.navigate(|b| b.extend_to(idx));
        } else {
            doc.navigate(|b| b.set_cursor(idx));
        }
    }
    if response.triple_clicked() {
        let idx = at(doc, view);
        doc.navigate(|b| b.select_line_at(idx));
    } else if response.double_clicked() {
        let idx = at(doc, view);
        doc.navigate(|b| b.select_word_at(idx));
    } else if response.dragged_by(PointerButton::Primary) && !mods.alt {
        let idx = at(doc, view);
        doc.navigate(|b| b.extend_to(idx));
        // Scroll while the pointer is held outside the view.
        let clip = ui.clip_rect();
        let dy = if pos.y < clip.top() {
            clip.top() - pos.y
        } else if pos.y > clip.bottom() {
            clip.bottom() - pos.y
        } else {
            0.0
        };
        if dy != 0.0 {
            ui.scroll_with_delta(vec2(0.0, dy.clamp(-40.0, 40.0)));
        }
        ui.ctx().request_repaint();
    }
}

fn char_at_point(
    doc: &Document,
    view: &View,
    m: &Metrics,
    text_x: f32,
    origin_y: f32,
    wrap_cols: usize,
    pos: egui::Pos2,
) -> usize {
    let buffer = doc.buffer();
    let lines = buffer.line_count();
    let row = (((pos.y - origin_y) / m.row_h).floor().max(0.0)) as usize;
    let total = total_rows(&view.cache, lines);
    if row >= total {
        return buffer.len_chars();
    }
    let line = line_of_row(&view.cache, lines, row);
    let laid = lay_cached(&view.cache, buffer, line, wrap_cols);
    let ri = (row - first_row_of(&view.cache, line)).min(laid.rows.len().saturating_sub(1));
    let (a, b) = laid.rows.get(ri).copied().unwrap_or((0, 0));
    let col = (pos.x - text_x) / m.cw;
    let idx = layout::char_at_col(&laid.cells, a, b, col);
    // A wrapped row's end is the next row's start: stay on this row.
    let idx = if ri + 1 < laid.rows.len() && idx >= b {
        b.saturating_sub(1).max(a)
    } else {
        idx
    };
    buffer.line_start(line) + idx
}

fn handle_keys(ui: &mut Ui, doc: &mut Document, view: &mut View, opts: &Options, out: &mut Output) {
    let events: Vec<Event> = ui.input(|i| i.events.clone());
    let pairs = if opts.auto_close_pairs {
        Pairs::DEFAULT
    } else {
        Pairs::NONE
    };
    let rules = indent_rules(doc.language().id);
    let mac = cfg!(target_os = "macos");
    let page = view.page_rows;
    for event in events {
        match event {
            Event::Text(text) => {
                for c in text.chars() {
                    if !c.is_control() {
                        doc.apply(|b| b.type_char(c, pairs));
                    }
                }
            }
            Event::Paste(text) => {
                if !text.is_empty() {
                    doc.apply(|b| b.paste(&text));
                }
            }
            Event::Copy => {
                out.copied = Some(doc.buffer().selected_text(true));
            }
            Event::Cut => {
                out.copied = Some(doc.buffer().selected_text(true));
                if doc.buffer().selections().iter().all(|s| s.is_empty()) {
                    doc.apply(Buffer::delete_lines);
                } else {
                    doc.apply(|b| b.insert_text(""));
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_event(doc, key, modifiers, mac, page, pairs, rules),
            _ => {}
        }
    }
}

fn key_event(
    doc: &mut Document,
    key: Key,
    mods: Modifiers,
    mac: bool,
    page: usize,
    pairs: Pairs,
    rules: IndentRules,
) {
    let shift = mods.shift;
    let word = if mac { mods.alt } else { mods.ctrl };
    let big = mods.command && !word || (!mac && mods.ctrl);
    let extend = shift;
    let go = |doc: &mut Document, motion: Motion| doc.navigate(|b| b.move_cursors(motion, extend));
    match key {
        Key::Enter if !mods.command => {
            doc.apply(|b| b.newline(rules));
        }
        Key::Tab => {
            if shift {
                doc.apply(Buffer::outdent);
            } else {
                doc.apply(Buffer::indent);
            }
        }
        Key::Backspace => {
            if mods.command || mods.ctrl {
                if mac && mods.command {
                    doc.apply(Buffer::delete_to_line_start);
                } else {
                    doc.apply(Buffer::delete_word_backward);
                }
            } else {
                doc.apply(|b| b.delete_backward(pairs));
            }
        }
        Key::Delete => {
            if mods.command || mods.ctrl {
                doc.apply(Buffer::delete_word_forward);
            } else {
                doc.apply(Buffer::delete_forward);
            }
        }
        Key::ArrowLeft if word => go(doc, Motion::WordLeft),
        Key::ArrowRight if word => go(doc, Motion::WordRight),
        Key::ArrowLeft if mac && mods.command => go(doc, Motion::LineStart),
        Key::ArrowRight if mac && mods.command => go(doc, Motion::LineEnd),
        Key::ArrowUp if mac && mods.command => go(doc, Motion::DocStart),
        Key::ArrowDown if mac && mods.command => go(doc, Motion::DocEnd),
        Key::ArrowUp if big => go(doc, Motion::ParagraphUp),
        Key::ArrowDown if big => go(doc, Motion::ParagraphDown),
        Key::ArrowLeft => go(doc, Motion::Left),
        Key::ArrowRight => go(doc, Motion::Right),
        Key::ArrowUp => go(doc, Motion::Up),
        Key::ArrowDown => go(doc, Motion::Down),
        Key::Home if big => go(doc, Motion::DocStart),
        Key::End if big => go(doc, Motion::DocEnd),
        Key::Home => go(doc, Motion::LineStart),
        Key::End => go(doc, Motion::LineEnd),
        Key::PageUp => go(doc, Motion::PageUp(page)),
        Key::PageDown => go(doc, Motion::PageDown(page)),
        _ => {}
    }
}
