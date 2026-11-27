// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Preferences window: a list of categories, a search that looks through every setting, and a
//! page of settings for each category. Every change is applied and saved as it is made.

use codigo_engine::settings::{
    Appearance, AutoSave, CursorStyle, Settings, MAX_FONT_SIZE, MAX_LINE_HEIGHT, MAX_UI_SCALE,
    MIN_FONT_SIZE, MIN_LINE_HEIGHT, MIN_UI_SCALE,
};
use codigo_engine::theme::{self, Colors, Rgb, Slot};
use codigo_syntax::Scope;
use egui::{Align, Color32, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::fonts::Family;
use crate::i18n::{self, tr, trf, Lang};
use crate::overlays::{heading, modal, Overlay, Shown};
use crate::syntax_theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Appearance,
    Colors,
    Editor,
    Files,
    Terminal,
    Tools,
    Advanced,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::Appearance,
        Page::Colors,
        Page::Editor,
        Page::Files,
        Page::Terminal,
        Page::Tools,
        Page::Advanced,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Appearance => tr("Appearance"),
            Page::Colors => tr("Colour theme"),
            Page::Editor => tr("Editor"),
            Page::Files => tr("Files and saving"),
            Page::Terminal => tr("Terminal"),
            Page::Tools => tr("Tools and assistant"),
            Page::Advanced => tr("Advanced"),
        }
    }
}

/// What the window remembers while it is open.
pub struct PrefsState {
    page: Page,
    query: String,
    /// The zoom being dragged; it is applied when the slider is let go.
    zoom: f32,
    font_open: bool,
    font_query: String,
    mono_only: bool,
    rulers: String,
    reset_armed: bool,
    /// A change is waiting to be written to disk (a slider or colour picker is still held).
    dirty: bool,
}

impl PrefsState {
    pub fn new(app: &mut App) -> PrefsState {
        app.scan_fonts();
        PrefsState {
            page: Page::Appearance,
            query: String::new(),
            zoom: app.settings.ui_scale,
            font_open: false,
            font_query: String::new(),
            mono_only: true,
            rulers: rulers_text(&app.settings.rulers),
            reset_armed: false,
            dirty: false,
        }
    }
}

fn rulers_text(rulers: &[u16]) -> String {
    rulers
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The columns a text names: numbers separated by commas or spaces, the rest ignored.
fn parse_rulers(text: &str) -> Vec<u16> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse::<u16>().ok())
        .collect()
}

enum Action {
    Close,
    OpenAssistant,
    ResetAll,
}

pub fn overlay(ctx: &egui::Context, app: &mut App, look: &Look, mut st: Box<PrefsState>) -> Shown {
    let mut changed = false;
    let mut action: Option<Action> = None;
    let height = (ctx.content_rect().height() - 150.0).clamp(300.0, 640.0);
    let closed = modal(ctx, look, "preferences", 900.0, false, |ui| {
        if heading(ui, look, tr("Preferences"), "prefs") {
            action = Some(Action::Close);
        }
        ui.add_space(6.0);
        let fonts = app.fonts.as_deref();
        let settings = &mut app.settings;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(190.0, height),
                Layout::top_down(Align::Min),
                |ui| {
                    cr::search_field(ui, look, &mut st.query, tr("Search settings"), 190.0);
                    ui.add_space(8.0);
                    let searching = !st.query.trim().is_empty();
                    for page in Page::ALL {
                        let selected = !searching && st.page == page;
                        if cr::nav_row(ui, look, page.label(), selected, false).clicked() {
                            st.page = page;
                            st.query.clear();
                        }
                    }
                },
            );
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_min_height(height);
                    egui::ScrollArea::vertical()
                        .id_salt("prefs-scroll")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut env = Env {
                                look,
                                s: settings,
                                fonts,
                                st: &mut st,
                                changed: false,
                                action: None,
                            };
                            pages(ui, &mut env);
                            changed |= env.changed;
                            if let Some(a) = env.action {
                                action = Some(a);
                            }
                        });
                },
            );
        });
    });
    if changed {
        st.dirty = true;
    }
    // A slider or a colour picker still being dragged saves once it is let go.
    if st.dirty && !ctx.input(|i| i.pointer.any_down()) {
        app.save_settings();
        st.dirty = false;
    }
    match action {
        Some(Action::OpenAssistant) => {
            crate::ai_ui::open_settings(app);
            return (Overlay::None, true);
        }
        Some(Action::ResetAll) => {
            let recent = std::mem::take(&mut app.settings.recent_projects);
            app.settings = Settings {
                recent_projects: recent,
                ..Settings::default()
            };
            i18n::set(Lang::from_setting(&app.settings.language));
            st.zoom = app.settings.ui_scale;
            st.rulers = rulers_text(&app.settings.rulers);
            st.reset_armed = false;
            app.save_settings();
        }
        Some(Action::Close) => {
            app.save_settings();
            return (Overlay::None, true);
        }
        None => {}
    }
    if closed {
        app.save_settings();
        (Overlay::None, true)
    } else {
        (Overlay::Preferences(st), false)
    }
}

/// Everything a page needs to draw and change settings.
struct Env<'a> {
    look: &'a Look,
    s: &'a mut Settings,
    fonts: Option<&'a [Family]>,
    st: &'a mut PrefsState,
    changed: bool,
    action: Option<Action>,
}

/// What is being drawn: the search words, the headings still waiting for their first row, and how
/// many rows were drawn, so a search that finds nothing can say so.
struct Cx {
    query: String,
    title: Option<&'static str>,
    section: Option<&'static str>,
    drawn: usize,
}

impl Cx {
    fn wants(&self, label: &str, hint: &str) -> bool {
        self.query.is_empty()
            || label.to_lowercase().contains(&self.query)
            || hint.to_lowercase().contains(&self.query)
    }

    /// Draw the headings that were waiting, now that a row is going to follow them.
    fn flush(&mut self, ui: &mut Ui, look: &Look) {
        let p = &look.palette;
        if let Some(title) = self.title.take() {
            ui.add_space(10.0);
            ui.label(
                RichText::new(title)
                    .strong()
                    .size(font_size::BODY + 4.0)
                    .color(color(p.text_primary)),
            );
        }
        if let Some(section) = self.section.take() {
            ui.add_space(16.0);
            ui.label(
                RichText::new(section)
                    .strong()
                    .size(font_size::BODY)
                    .color(color(p.text_secondary)),
            );
            ui.add_space(2.0);
            ui.separator();
        }
        self.drawn += 1;
    }
}

fn pages(ui: &mut Ui, env: &mut Env) {
    let query = env.st.query.trim().to_lowercase();
    let searching = !query.is_empty();
    let mut cx = Cx {
        query,
        title: None,
        section: None,
        drawn: 0,
    };
    let list: Vec<Page> = if searching {
        Page::ALL.to_vec()
    } else {
        vec![env.st.page]
    };
    for page in list {
        cx.title = Some(page.label());
        if !searching {
            // A page shows its own name at once, even before its first row.
            cx.flush(ui, env.look);
            cx.drawn = 0;
        }
        cx.section = None;
        match page {
            Page::Appearance => appearance(ui, env, &mut cx),
            Page::Colors => colors(ui, env, &mut cx),
            Page::Editor => editor(ui, env, &mut cx),
            Page::Files => files(ui, env, &mut cx),
            Page::Terminal => terminal(ui, env, &mut cx),
            Page::Tools => tools(ui, env, &mut cx),
            Page::Advanced => advanced(ui, env, &mut cx),
        }
        cx.title = None;
    }
    if searching && cx.drawn == 0 {
        ui.add_space(20.0);
        ui.label(
            RichText::new(trf("No settings match “{}”.", &[env.st.query.trim()]))
                .color(color(env.look.palette.text_secondary)),
        );
    }
}

fn section(cx: &mut Cx, title: &'static str) {
    cx.section = Some(tr(title));
}

/// One setting: its name and a line saying what it does on the left, its control on the right.
/// Returns whether the control changed something.
fn row(
    cx: &mut Cx,
    ui: &mut Ui,
    look: &Look,
    label: &'static str,
    hint: &'static str,
    add: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let (label, hint) = (tr(label), tr(hint));
    if !cx.wants(label, hint) {
        return false;
    }
    cx.flush(ui, look);
    let p = &look.palette;
    ui.add_space(6.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(w * 0.55, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.label(RichText::new(label).color(color(p.text_primary)));
                if !hint.is_empty() {
                    ui.label(
                        RichText::new(hint)
                            .size(font_size::BODY - 1.0)
                            .color(color(p.text_tertiary)),
                    );
                }
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = add(ui);
        });
    });
    changed
}

/// A block that spans the page, such as a grid of themes. `words` are extra things a search for
/// it should find.
fn block(
    cx: &mut Cx,
    ui: &mut Ui,
    look: &Look,
    label: &'static str,
    words: &'static str,
    add: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let (label, words) = (tr(label), tr(words));
    if !cx.wants(label, words) {
        return false;
    }
    cx.flush(ui, look);
    ui.add_space(8.0);
    add(ui)
}

/// Buttons that pick one of several values.
fn choice<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    // The row lays out right to left, so the options go in backwards to read in order.
    for (v, label) in options.iter().rev() {
        if ui.selectable_label(*value == *v, *label).clicked() && *value != *v {
            *value = *v;
            changed = true;
        }
    }
    changed
}

fn switch(ui: &mut Ui, value: &mut bool) -> bool {
    ui.checkbox(value, "").changed()
}

fn text_field(ui: &mut Ui, look: &Look, text: &mut String, hint: &str, width: f32) -> bool {
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .desired_width(width),
            )
            .changed()
        })
        .inner
}

fn slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> egui::Response {
    ui.add(
        egui::Slider::new(value, range)
            .suffix(suffix.to_owned())
            .max_decimals(2),
    )
}

// --- Appearance -----------------------------------------------------------------------------

fn appearance(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    let s = &mut *env.s;
    env.changed |= row(
        cx,
        ui,
        look,
        "Window theme",
        "Follow the system, or always light or dark.",
        |ui| {
            choice(
                ui,
                &mut s.appearance,
                &[
                    (Appearance::System, tr("System")),
                    (Appearance::Light, tr("Light")),
                    (Appearance::Dark, tr("Dark")),
                ],
            )
        },
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Language",
        "The language of the interface.",
        |ui| {
            let mut code: &'static str = match s.language.as_str() {
                "es" => "es",
                "en" => "en",
                _ => "auto",
            };
            let changed = choice(
                ui,
                &mut code,
                &[("auto", tr("System")), ("es", "Español"), ("en", "English")],
            );
            if changed {
                i18n::set(Lang::from_setting(code));
                s.language = code.to_owned();
            }
            changed
        },
    );
    let st = &mut *env.st;
    let mut zoom_applied = false;
    env.changed |= row(
        cx,
        ui,
        look,
        "Interface zoom",
        "Makes the whole window bigger or smaller.",
        |ui| {
            let r = ui.add(
                egui::Slider::new(&mut st.zoom, MIN_UI_SCALE..=MAX_UI_SCALE)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .custom_parser(|t| {
                        t.trim()
                            .trim_end_matches('%')
                            .parse::<f64>()
                            .ok()
                            .map(|v| v / 100.0)
                    }),
            );
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                zoom_applied = true;
            }
            zoom_applied
        },
    );
    if zoom_applied {
        s.ui_scale = st.zoom;
    }
    env.changed |= row(
        cx,
        ui,
        look,
        "High contrast",
        "Stronger borders and text, for easier reading.",
        |ui| switch(ui, &mut s.high_contrast),
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Reduce motion",
        "Turns the animations off.",
        |ui| switch(ui, &mut s.reduced_motion),
    );
    if crate::chrome::NDL_CONTROLS_POSSIBLE {
        env.changed |= row(
            cx,
            ui,
            look,
            "Use the desktop's own window buttons",
            "Instead of the round Nodalia ones. Takes effect when Código restarts.",
            |ui| switch(ui, &mut s.native_window_controls),
        );
    }
}

// --- Colours --------------------------------------------------------------------------------

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn slot_label(slot: Slot) -> &'static str {
    match slot {
        Slot::Background => tr("Background"),
        Slot::Text => tr("Text"),
        Slot::LineNumbers => tr("Line numbers"),
        Slot::Selection => tr("Selection"),
        Slot::Cursor => tr("Cursor"),
        Slot::ActiveLine => tr("Current line"),
        Slot::Syntax(scope) => match scope {
            Scope::Keyword => tr("Keywords"),
            Scope::Control => tr("Control flow"),
            Scope::Operator => tr("Operators"),
            Scope::Punctuation => tr("Punctuation"),
            Scope::Bracket => tr("Brackets"),
            Scope::String => tr("Strings"),
            Scope::Escape => tr("Escape sequences"),
            Scope::Number => tr("Numbers"),
            Scope::Constant => tr("Constants"),
            Scope::Boolean => tr("Booleans"),
            Scope::Comment => tr("Comments"),
            Scope::DocComment => tr("Documentation comments"),
            Scope::Function => tr("Functions"),
            Scope::Method => tr("Methods"),
            Scope::Macro => tr("Macros"),
            Scope::Type => tr("Types"),
            Scope::BuiltinType => tr("Built-in types"),
            Scope::Variable => tr("Variables"),
            Scope::Parameter => tr("Parameters"),
            Scope::Property => tr("Properties"),
            Scope::Attribute => tr("Attributes"),
            Scope::Namespace => tr("Namespaces"),
            Scope::Label => tr("Labels"),
            Scope::Tag => tr("Tags"),
            Scope::Heading => tr("Headings"),
            Scope::Emphasis => tr("Emphasis"),
            Scope::Link => tr("Links"),
            Scope::Literal => tr("Other literals"),
        },
    }
}

fn colors(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    section(cx, "Preview");
    {
        let current = syntax_theme::editor_colors(env.s, &look.palette);
        let (size, line_height) = (env.s.font_size, env.s.line_height);
        let style = env.s.cursor_style;
        block(cx, ui, look, "Preview", "", |ui| {
            preview(ui, look, &current, size, line_height, style);
            false
        });
    }
    section(cx, "Theme");
    {
        let s = &mut *env.s;
        env.changed |= block(
            cx,
            ui,
            look,
            "Colour theme",
            "dracula nord solarized monokai one dark gruvbox",
            |ui| theme_cards(ui, look, s),
        );
    }
    section(cx, "Editor colours");
    for slot in Slot::all().take_while(|s| !matches!(s, Slot::Syntax(_))) {
        env.changed |= slot_row(ui, env, cx, slot);
    }
    section(cx, "Code colours");
    for slot in Slot::all().filter(|s| matches!(s, Slot::Syntax(_))) {
        env.changed |= slot_row(ui, env, cx, slot);
    }
    section(cx, "Reset");
    let look = env.look;
    let any = !env.s.color_overrides.is_empty();
    let s = &mut *env.s;
    env.changed |= row(
        cx,
        ui,
        look,
        "Reset colours",
        "Goes back to the colours of the theme.",
        |ui| {
            if cr::secondary_button(ui, look, tr("Reset colours"), any).clicked() {
                s.color_overrides.clear();
                true
            } else {
                false
            }
        },
    );
}

/// One colour of the editor, with a swatch to change it and a button to take the change back.
fn slot_row(ui: &mut Ui, env: &mut Env, cx: &mut Cx, slot: Slot) -> bool {
    let look = env.look;
    let current = syntax_theme::editor_colors(env.s, &look.palette).get(slot);
    let key = slot.key();
    let overridden = env.s.color_overrides.contains_key(&key);
    let label = slot_label(slot);
    let s = &mut *env.s;
    // The rows are written out with their own label, so one function serves them all.
    let (label_text, hint) = (label, "");
    if !cx.wants(label_text, hint) {
        return false;
    }
    cx.flush(ui, look);
    let mut changed = false;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut rgb = [current.0, current.1, current.2];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            s.color_overrides
                .insert(key.clone(), Rgb(rgb[0], rgb[1], rgb[2]).hex());
            changed = true;
        }
        ui.label(RichText::new(label).color(color(look.palette.text_primary)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if overridden && cr::secondary_button(ui, look, tr("Reset"), true).clicked() {
                s.color_overrides.remove(&key);
                changed = true;
            }
            ui.label(
                RichText::new(current.hex())
                    .monospace()
                    .size(font_size::BODY - 1.0)
                    .color(color(look.palette.text_tertiary)),
            );
        });
    });
    changed
}

/// The grid of themes. Picking a dark or light theme also moves the window to match.
fn theme_cards(ui: &mut Ui, look: &Look, s: &mut Settings) -> bool {
    let p = &look.palette;
    let mut changed = false;
    let mut entries: Vec<(&str, &str, Colors, Option<bool>)> = vec![(
        theme::NODALIA,
        "Nodalia",
        syntax_theme::nodalia_colors(p),
        None,
    )];
    for preset in theme::PRESETS {
        entries.push((preset.id, preset.name, preset.colors(), Some(preset.dark)));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        for (id, name, colors, dark) in entries {
            let selected = s.color_theme == id;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(150.0, 98.0), Sense::click());
            let painter = ui.painter();
            let radius = CornerRadius::same(8);
            painter.rect_filled(rect, radius, c32(colors.get(Slot::Background)));
            let stroke = if selected {
                Stroke::new(2.0, color(p.accent_default))
            } else if resp.hovered() {
                Stroke::new(1.5, color(p.text_secondary))
            } else {
                Stroke::new(1.0, color(p.border_default))
            };
            painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
            // A few strokes in the theme's own colours stand for code.
            let lines: [(f32, f32, Scope); 4] = [
                (0.0, 46.0, Scope::Keyword),
                (14.0, 70.0, Scope::Function),
                (14.0, 54.0, Scope::String),
                (0.0, 30.0, Scope::Comment),
            ];
            for (i, (indent, width, scope)) in lines.into_iter().enumerate() {
                let y = rect.top() + 14.0 + i as f32 * 12.0;
                let line = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 12.0 + indent, y),
                    egui::vec2(width, 5.0),
                );
                painter.rect_filled(line, CornerRadius::same(2), c32(colors.scope(scope)));
            }
            painter.text(
                egui::pos2(rect.left() + 12.0, rect.bottom() - 10.0),
                egui::Align2::LEFT_BOTTOM,
                name,
                FontId::proportional(font_size::BODY - 1.0),
                c32(colors.get(Slot::Text)),
            );
            if resp.clicked() && !selected {
                s.color_theme = id.to_owned();
                // The overrides were made for the old theme; they would only recolour the new one.
                s.color_overrides.clear();
                if let Some(dark) = dark {
                    s.appearance = if dark {
                        Appearance::Dark
                    } else {
                        Appearance::Light
                    };
                }
                changed = true;
            }
        }
    });
    changed
}

/// A few lines of code drawn the way the editor draws them.
fn preview(
    ui: &mut Ui,
    look: &Look,
    colors: &Colors,
    size: f32,
    line_height: f32,
    style: CursorStyle,
) {
    type Tokens = Vec<(&'static str, Option<Scope>)>;
    let lines: Vec<Tokens> = vec![
        vec![("// Greets the world", Some(Scope::Comment))],
        vec![
            ("fn", Some(Scope::Keyword)),
            (" ", None),
            ("main", Some(Scope::Function)),
            ("()", Some(Scope::Punctuation)),
            (" {", Some(Scope::Punctuation)),
        ],
        vec![
            ("    ", None),
            ("let", Some(Scope::Keyword)),
            (" total", Some(Scope::Variable)),
            (": ", Some(Scope::Punctuation)),
            ("usize", Some(Scope::BuiltinType)),
            (" = ", Some(Scope::Operator)),
            ("42", Some(Scope::Number)),
            (";", Some(Scope::Punctuation)),
        ],
        vec![
            ("    ", None),
            ("println!", Some(Scope::Macro)),
            ("(", Some(Scope::Bracket)),
            ("\"Hello, {}!\"", Some(Scope::String)),
            (", total", None),
            (")", Some(Scope::Bracket)),
            (";", Some(Scope::Punctuation)),
        ],
        vec![("}", Some(Scope::Punctuation))],
    ];
    let row_h = (size * line_height).max(12.0);
    let pad = 10.0;
    let gutter = 34.0;
    let height = pad * 2.0 + row_h * lines.len() as f32;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(8),
        c32(colors.get(Slot::Background)),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, color(look.palette.border_default)),
        egui::StrokeKind::Inside,
    );
    let font = FontId::monospace(size);
    for (i, tokens) in lines.iter().enumerate() {
        let y = rect.top() + pad + i as f32 * row_h;
        let band = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, y),
            egui::vec2(rect.width() - 2.0, row_h),
        );
        if i == 2 {
            painter.rect_filled(band, CornerRadius::ZERO, c32(colors.get(Slot::ActiveLine)));
        }
        painter.text(
            egui::pos2(rect.left() + gutter - 8.0, y + row_h / 2.0),
            egui::Align2::RIGHT_CENTER,
            (i + 1).to_string(),
            FontId::monospace((size - 1.0).max(8.0)),
            c32(colors.get(Slot::LineNumbers)),
        );
        let mut job = egui::text::LayoutJob::default();
        for (text, scope) in tokens {
            let ink = scope.map_or_else(|| colors.get(Slot::Text), |s| colors.scope(s));
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: c32(ink),
                    italics: scope.is_some_and(theme::italic),
                    ..Default::default()
                },
            );
        }
        let galley = painter.layout_job(job);
        let origin = egui::pos2(rect.left() + gutter, y + (row_h - galley.size().y) / 2.0);
        let end = origin.x + galley.size().x;
        if i == 2 {
            // The selection covers the number, and the cursor sits after it.
            let sel = egui::Rect::from_min_size(
                egui::pos2(origin.x + galley.size().x * 0.62, y + 1.0),
                egui::vec2(galley.size().x * 0.1, row_h - 2.0),
            );
            painter.rect_filled(sel, CornerRadius::ZERO, c32(colors.get(Slot::Selection)));
        }
        painter.galley(origin, galley, c32(colors.get(Slot::Text)));
        if i == 2 {
            let cursor = c32(colors.get(Slot::Cursor));
            let cw = size * 0.6;
            let r = match style {
                CursorStyle::Line => egui::Rect::from_min_size(
                    egui::pos2(end, y + 2.0),
                    egui::vec2(2.0, row_h - 4.0),
                ),
                CursorStyle::Block => {
                    egui::Rect::from_min_size(egui::pos2(end, y + 2.0), egui::vec2(cw, row_h - 4.0))
                }
                CursorStyle::Underline => {
                    egui::Rect::from_min_size(egui::pos2(end, y + row_h - 4.0), egui::vec2(cw, 2.0))
                }
            };
            painter.rect_filled(r, CornerRadius::ZERO, cursor);
        }
    }
}

// --- Editor ---------------------------------------------------------------------------------

fn editor(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    section(cx, "Font");
    font_row(ui, env, cx);
    let s = &mut *env.s;
    env.changed |= row(
        cx,
        ui,
        look,
        "Font size",
        "The size of the code, in points.",
        |ui| slider(ui, &mut s.font_size, MIN_FONT_SIZE..=MAX_FONT_SIZE, " pt").changed(),
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Line height",
        "How much room each line takes, as a multiple of the font size.",
        |ui| {
            slider(
                ui,
                &mut s.line_height,
                MIN_LINE_HEIGHT..=MAX_LINE_HEIGHT,
                "×",
            )
            .changed()
        },
    );
    {
        let current = syntax_theme::editor_colors(s, &look.palette);
        let (size, lh, style) = (s.font_size, s.line_height, s.cursor_style);
        block(cx, ui, look, "Preview", "font sample", |ui| {
            preview(ui, look, &current, size, lh, style);
            false
        });
    }

    section(cx, "Cursor");
    env.changed |= row(cx, ui, look, "Cursor style", "", |ui| {
        choice(
            ui,
            &mut s.cursor_style,
            &[
                (CursorStyle::Line, tr("Line")),
                (CursorStyle::Block, tr("Block")),
                (CursorStyle::Underline, tr("Underline")),
            ],
        )
    });
    env.changed |= row(cx, ui, look, "Blinking cursor", "", |ui| {
        switch(ui, &mut s.cursor_blink)
    });

    section(cx, "Display");
    env.changed |= row(cx, ui, look, "Line numbers", "", |ui| {
        switch(ui, &mut s.line_numbers)
    });
    env.changed |= row(cx, ui, look, "Highlight the current line", "", |ui| {
        switch(ui, &mut s.highlight_active_line)
    });
    env.changed |= row(
        cx,
        ui,
        look,
        "Word wrap",
        "Long lines continue on the next row instead of scrolling sideways.",
        |ui| switch(ui, &mut s.word_wrap),
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Show whitespace",
        "Spaces and tabs as dots and arrows.",
        |ui| switch(ui, &mut s.show_whitespace),
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Show the minimap",
        "An overview of the whole file beside the editor.",
        |ui| switch(ui, &mut s.minimap),
    );
    env.changed |= row(
        cx,
        ui,
        look,
        "Indent guides",
        "Thin vertical lines at each level of indentation.",
        |ui| switch(ui, &mut s.indent_guides),
    );
    {
        let st = &mut *env.st;
        let mut edited = false;
        env.changed |= row(
            cx,
            ui,
            look,
            "Rulers",
            "Columns where a thin line is drawn, such as 80, 100.",
            |ui| {
                if text_field(ui, look, &mut st.rulers, "80, 100", 140.0) {
                    s.rulers = parse_rulers(&st.rulers);
                    edited = true;
                }
                edited
            },
        );
    }
    env.changed |= row(
        cx,
        ui,
        look,
        "Show who changed the line",
        "Who last changed the line the cursor is on, at the end of the line.",
        |ui| switch(ui, &mut s.blame),
    );

    section(cx, "Typing");
    env.changed |= row(
        cx,
        ui,
        look,
        "Close brackets and quotes automatically",
        "",
        |ui| switch(ui, &mut s.auto_close_pairs),
    );
}

/// The code font: the one in use, and a list of the installed ones to pick from.
fn font_row(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    let current = if env.s.font_family.is_empty() {
        tr("Built-in").to_owned()
    } else {
        env.s.font_family.clone()
    };
    let st = &mut *env.st;
    let open = st.font_open;
    let mut toggle = false;
    row(
        cx,
        ui,
        look,
        "Code font",
        "Any font installed on this computer. Fixed-width ones are the best for code.",
        |ui| {
            let label = format!("{current}  ▾");
            toggle = cr::secondary_button(ui, look, &label, true).clicked();
            false
        },
    );
    if toggle {
        st.font_open = !open;
    }
    if !st.font_open
        || !cx.wants(
            tr("Code font"),
            tr("Any font installed on this computer. Fixed-width ones are the best for code."),
        )
    {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        cr::search_field(ui, look, &mut st.font_query, tr("Search fonts"), 240.0);
        ui.checkbox(&mut st.mono_only, tr("Fixed-width only"));
        if !env.s.font_family.is_empty()
            && cr::secondary_button(ui, look, tr("Use the built-in font"), true).clicked()
        {
            env.s.font_family.clear();
            env.changed = true;
        }
    });
    ui.add_space(4.0);
    let Some(fonts) = env.fonts else {
        ui.label(
            RichText::new(tr("Looking for the installed fonts…"))
                .color(color(look.palette.text_secondary)),
        );
        return;
    };
    let q = st.font_query.trim().to_lowercase();
    let shown: Vec<&Family> = fonts
        .iter()
        .filter(|f| {
            (!st.mono_only || f.mono) && (q.is_empty() || f.name.to_lowercase().contains(&q))
        })
        .collect();
    if shown.is_empty() {
        ui.label(RichText::new(tr("No font matches.")).color(color(look.palette.text_secondary)));
        return;
    }
    cr::field_frame(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical()
            .id_salt("prefs-fonts")
            .max_height(220.0)
            .show_rows(ui, 26.0, shown.len(), |ui, range| {
                for family in shown.get(range).into_iter().flatten() {
                    let selected = env.s.font_family == family.name;
                    let resp = ui.selectable_label(selected, family.name.as_str());
                    if resp.clicked() {
                        env.s.font_family = family.name.clone();
                        env.changed = true;
                    }
                }
            });
    });
}

// --- Files ----------------------------------------------------------------------------------

fn files(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    let s = &mut *env.s;
    section(cx, "Saving");
    env.changed |= row(
        cx,
        ui,
        look,
        "Auto save",
        "Saves changed files by itself.",
        |ui| {
            choice(
                ui,
                &mut s.auto_save,
                &[
                    (AutoSave::Off, tr("Off")),
                    (AutoSave::OnFocusLoss, tr("When the window loses focus")),
                    (AutoSave::AfterDelay, tr("After a delay")),
                ],
            )
        },
    );
    if s.auto_save == AutoSave::AfterDelay {
        let mut seconds = s.auto_save_delay_ms as f32 / 1000.0;
        env.changed |= row(
            cx,
            ui,
            look,
            "Auto save delay",
            "How long after the last edit.",
            |ui| {
                let r = slider(ui, &mut seconds, 0.2..=60.0, " s");
                if r.changed() {
                    s.auto_save_delay_ms = (seconds * 1000.0).round() as u64;
                }
                r.changed()
            },
        );
    }
    env.changed |= row(cx, ui, look, "Trim trailing whitespace on save", "", |ui| {
        switch(ui, &mut s.trim_trailing_whitespace_on_save)
    });
    env.changed |= row(cx, ui, look, "Insert a final newline on save", "", |ui| {
        switch(ui, &mut s.insert_final_newline_on_save)
    });
}

// --- Terminal -------------------------------------------------------------------------------

fn terminal(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    let s = &mut *env.s;
    section(cx, "Shell");
    env.changed |= row(
        cx,
        ui,
        look,
        "Shell program",
        "A path or a name such as fish or zsh. Empty uses your login shell.",
        |ui| text_field(ui, look, &mut s.terminal_shell, tr("Login shell"), 220.0),
    );
}

// --- Tools ----------------------------------------------------------------------------------

fn tools(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    let s = &mut *env.s;
    section(cx, "Language servers");
    env.changed |= row(
        cx,
        ui,
        look,
        "Use language servers for diagnostics when one is installed",
        "Errors, completions and go to definition.",
        |ui| switch(ui, &mut s.language_servers),
    );
    section(cx, "Assistant");
    let provider = s.ai_provider.clone();
    let mut open = false;
    row(
        cx,
        ui,
        look,
        "Assistant provider and key",
        "Which model answers, and where. Keys are kept in the system's keyring.",
        |ui| {
            let label = format!("{provider}  ·  {}", tr("Configure…"));
            open = cr::secondary_button(ui, look, &label, true).clicked();
            false
        },
    );
    if open {
        env.action = Some(Action::OpenAssistant);
    }
    section(cx, "Updates");
    env.changed |= row(
        cx,
        ui,
        look,
        "Check for updates when Código starts",
        "",
        |ui| switch(ui, &mut s.check_updates),
    );
}

// --- Advanced -------------------------------------------------------------------------------

fn advanced(ui: &mut Ui, env: &mut Env, cx: &mut Cx) {
    let look = env.look;
    section(cx, "Settings file");
    let path = Settings::file().map_or_else(String::new, |p| p.display().to_string());
    block(
        cx,
        ui,
        look,
        "Settings file",
        "settings.json location",
        |ui| {
            ui.label(
                RichText::new(tr(
                    "Everything here is saved in this file. You can also edit it by hand.",
                ))
                .color(color(look.palette.text_secondary)),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(path)
                    .monospace()
                    .size(font_size::BODY - 1.0)
                    .color(color(look.palette.text_tertiary)),
            );
            false
        },
    );
    section(cx, "Reset");
    let armed = env.st.reset_armed;
    let mut clicked = false;
    row(
        cx,
        ui,
        look,
        "Reset all preferences",
        "Goes back to how Código was when it was installed. Your recent projects are kept.",
        |ui| {
            let label = if armed {
                tr("Click again to reset everything")
            } else {
                tr("Reset all preferences")
            };
            clicked = cr::secondary_button(ui, look, label, true).clicked();
            false
        },
    );
    if clicked {
        if armed {
            env.action = Some(Action::ResetAll);
        } else {
            env.st.reset_armed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rulers_come_from_any_list_of_numbers() {
        assert_eq!(parse_rulers("80, 100"), vec![80, 100]);
        assert_eq!(parse_rulers("80 100;120"), vec![80, 100, 120]);
        assert_eq!(parse_rulers(""), Vec::<u16>::new());
        assert_eq!(parse_rulers("abc, -5, 99999999"), vec![5]);
        assert_eq!(rulers_text(&[80, 100]), "80, 100");
    }

    #[test]
    fn every_page_has_a_name() {
        for page in Page::ALL {
            assert!(!page.label().is_empty());
        }
    }

    #[test]
    fn every_colour_slot_has_a_label() {
        let mut seen = std::collections::HashSet::new();
        for slot in Slot::all() {
            assert!(seen.insert(slot_label(slot)), "{slot:?} shares a label");
        }
    }
}
