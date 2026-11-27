// SPDX-License-Identifier: MIT OR Apache-2.0
//! The assistant: a panel to talk with a model about the open code, commands that ask it to
//! explain, fix, refactor, document or test the selection, and the settings (provider, model,
//! key). Nothing is sent until the person asks; the key lives in the system's keyring and goes
//! only to the provider's own host. Every call runs on a worker thread and comes back as an
//! [`AiMsg`].

use std::collections::HashMap;
use std::ops::Range;

use std::path::PathBuf;

use codigo_ai::context::{self, Active, Context};
use codigo_ai::edits::{self, Change, FileEdit, Locate};
use codigo_ai::prompts::{self, Task};
use codigo_ai::{Config, Message, Provider};
use codigo_engine::DocId;
use codigo_text::Edit;
use egui::{Align2, FontId, RichText, ScrollArea, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg, Panel};
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};

/// How many earlier turns go along with a new question.
const HISTORY_TURNS: usize = 8;

/// Where the code of an answer goes if the person applies it: the text it was asked about.
#[derive(Clone, Debug)]
pub struct Target {
    pub doc: DocId,
    pub range: Range<usize>,
    /// The revision of the text when it was asked about; a changed file is never overwritten.
    pub revision: u64,
    pub file: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
    /// An error from the provider or the setup, shown in the conversation.
    Problem,
}

/// Where a proposed change stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stand {
    Pending,
    Applied,
    Rejected,
    /// Why it could not be applied.
    Failed(String),
}

/// A change the assistant proposed to a file, shown as a diff the person accepts or rejects.
pub struct Proposal {
    pub edit: FileEdit,
    pub stand: Stand,
}

pub struct Turn {
    /// Tells a turn apart when the list changes under an answer that is still on its way.
    pub uid: u64,
    pub edits: Vec<Proposal>,
    pub role: Who,
    /// What the panel shows.
    pub shown: String,
    /// What was sent (a task turn carries the code).
    pub sent: String,
    pub target: Option<Target>,
}

#[derive(Default)]
pub struct AiPanel {
    pub turns: Vec<Turn>,
    pub input: String,
    pub with_selection: bool,
    pub busy: bool,
    /// The answer that is awaited; older answers are dropped.
    pub serial: u64,
    /// Keys that could not go to a keyring, kept for this session only.
    pub session_keys: HashMap<Provider, String>,
    pub focus: bool,
    pub scroll_down: bool,
    next_uid: u64,
}

impl AiPanel {
    fn fresh_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }
}

pub enum AiMsg {
    Reply {
        serial: u64,
        target: Option<Target>,
        result: Result<String, String>,
    },
    /// Whether a key is saved for the provider.
    HasKey(Provider, bool),
    /// A key was kept, or why not.
    KeyStored(Provider, String, Result<(), String>),
    KeyRemoved(Provider, Result<(), String>),
    /// A file the assistant proposed was written, or why not.
    Created {
        uid: u64,
        index: usize,
        path: PathBuf,
        result: Result<(), String>,
    },
}

fn config(app: &App) -> Config {
    let provider = Provider::from_id(&app.settings.ai_provider);
    Config {
        provider,
        model: if app.settings.ai_model.trim().is_empty() {
            provider.default_model().to_owned()
        } else {
            app.settings.ai_model.clone()
        },
        base_url: app.settings.ai_base_url.clone(),
    }
}

fn spawn(app: &App, work: impl FnOnce() -> AiMsg + Send + 'static) {
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-ai".into())
        .spawn(move || {
            let _ = tx.send(Msg::Ai(work()));
            ctx.request_repaint();
        });
}

pub fn show(app: &mut App) {
    app.sidebar_visible = true;
    app.panel = Panel::Assistant;
    app.ai.focus = true;
}

/// Ask for a job on the selection (or the whole file when nothing is selected).
pub fn task(app: &mut App, task: Task) {
    let Some(tab) = app.tabs.get(app.active) else {
        app.toast(tr("Open a file first."));
        return;
    };
    let buffer = tab.doc.buffer();
    let sel = buffer.primary();
    let range = if sel.start() == sel.end() {
        0..buffer.len_chars()
    } else {
        sel.start()..sel.end()
    };
    if range.is_empty() {
        app.toast(tr("There is no code to ask about."));
        return;
    }
    let code = buffer.slice(range.clone());
    let file = tab
        .doc
        .file_name()
        .unwrap_or_else(|| tr("Untitled").to_owned());
    let language = tab.doc.language().id;
    let target = Target {
        doc: tab.doc.id(),
        range,
        revision: buffer.revision(),
        file: file.clone(),
    };
    let verb = match task {
        Task::Explain => tr("Explain"),
        Task::Fix => tr("Fix"),
        Task::Refactor => tr("Refactor"),
        Task::Document => tr("Document"),
        Task::Test => tr("Write tests for"),
    };
    let shown = format!("{verb} {file}");
    let sent = prompts::ask(task, &file, language, &code);
    show(app);
    let gathered = gather(app, &file);
    send(app, shown, sent, Some(target), gathered);
}

/// What the person typed in the panel, with the selected code when they ticked the box.
fn ask_from_input(app: &mut App) {
    let typed = app.ai.input.trim().to_owned();
    if typed.is_empty() || app.ai.busy {
        return;
    }
    let mut sent = typed.clone();
    let mut target = None;
    if app.ai.with_selection {
        if let Some(tab) = app.tabs.get(app.active) {
            let b = tab.doc.buffer();
            let sel = b.primary();
            if sel.start() != sel.end() {
                let code = b.slice(sel.start()..sel.end());
                let file = tab.doc.file_name().unwrap_or_default();
                sent = format!(
                    "{typed}\n\nFile: {file}\n\n{}",
                    prompts::fenced(tab.doc.language().id, &code)
                );
                target = Some(Target {
                    doc: tab.doc.id(),
                    range: sel.start()..sel.end(),
                    revision: b.revision(),
                    file,
                });
            }
        }
    }
    app.ai.input.clear();
    let gathered = gather(app, &typed);
    send(app, typed, sent, target, gathered);
}

/// What the assistant is told about the project, built on the UI thread from what is open now.
struct Gathered {
    context: Context,
    /// Mentioned files that are not open: read from disk off the UI thread.
    from_disk: Vec<(String, PathBuf)>,
}

/// The longest file name or path kept in the context line.
const OPEN_TABS: usize = 20;

fn gather(app: &App, typed: &str) -> Option<Gathered> {
    if !app.settings.ai_context {
        return None;
    }
    let root = app.project.as_ref().map(|p| p.root.clone());
    let files: Vec<String> = app
        .files
        .as_ref()
        .map(|(_, f)| f.as_ref().clone())
        .unwrap_or_default();
    let relative = |path: &std::path::Path| -> String {
        root.as_ref()
            .and_then(|r| path.strip_prefix(r).ok())
            .map_or_else(
                || path.display().to_string(),
                |p| p.to_string_lossy().replace('\\', "/"),
            )
    };
    let active = app.tabs.get(app.active).map(|tab| {
        let b = tab.doc.buffer();
        let sel = b.primary();
        let len = b.len_chars();
        // A huge file goes as the stretch around the caret, not whole.
        let (text, base) = if len <= 400_000 {
            (b.slice(0..len), 0)
        } else {
            let lo = sel.head.saturating_sub(20_000);
            (b.slice(lo..(sel.head + 20_000).min(len)), lo)
        };
        let selection = (sel.start() != sel.end() && sel.end() - sel.start() <= 40_000)
            .then(|| (b.line_of(sel.start()) + 1, b.slice(sel.start()..sel.end())));
        Active {
            path: tab
                .doc
                .path()
                .map_or_else(|| tab.doc.file_name().unwrap_or_default(), relative),
            language: tab.doc.language().id.to_owned(),
            text,
            cursor_line: b.line_of(sel.head) - if base == 0 { 0 } else { b.line_of(base) } + 1,
            selection,
        }
    });
    let open: Vec<String> = app
        .tabs
        .iter()
        .take(OPEN_TABS)
        .filter_map(|t| t.doc.file_name())
        .collect();
    let mut attached = Vec::new();
    let mut from_disk = Vec::new();
    if let Some(root) = &root {
        for rel in context::mentions(typed, &files) {
            let abs = root.join(&rel);
            match app
                .tabs
                .iter()
                .find(|t| t.doc.path() == Some(abs.as_path()))
            {
                Some(tab) => {
                    let b = tab.doc.buffer();
                    attached.push((rel, b.slice(0..b.len_chars())));
                }
                None => from_disk.push((rel, abs)),
            }
        }
    }
    Some(Gathered {
        context: Context {
            project: root
                .as_ref()
                .and_then(|r| r.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            files,
            open,
            active,
            attached,
        },
        from_disk,
    })
}

/// A line saying what the next question will carry, so nothing is sent by surprise.
fn context_line(app: &App) -> String {
    if !app.settings.ai_context {
        return tr("Only what you type is sent.").to_owned();
    }
    let mut parts = Vec::new();
    if let Some(tab) = app.tabs.get(app.active) {
        parts.push(
            tab.doc
                .file_name()
                .unwrap_or_else(|| tr("Untitled").to_owned()),
        );
        let sel = tab.doc.buffer().primary();
        if sel.start() != sel.end() {
            parts.push(tr("the selection").to_owned());
        }
    }
    if app.files.is_some() {
        parts.push(tr("the project's file list").to_owned());
    }
    if parts.is_empty() {
        return tr("Open a file and the assistant can see it.").to_owned();
    }
    trf("The assistant sees: {}.", &[&parts.join(" · ")])
}

fn send(
    app: &mut App,
    shown: String,
    sent: String,
    target: Option<Target>,
    gathered: Option<Gathered>,
) {
    if app.ai.busy {
        app.toast(tr("The assistant is still answering."));
        return;
    }
    let cfg = config(app);
    let uid = app.ai.fresh_uid();
    app.ai.turns.push(Turn {
        uid,
        edits: Vec::new(),
        role: Who::User,
        shown,
        sent,
        target: target.clone(),
    });
    app.ai.busy = true;
    app.ai.serial += 1;
    app.ai.scroll_down = true;
    let serial = app.ai.serial;
    let skip = app.ai.turns.len().saturating_sub(HISTORY_TURNS);
    let messages: Vec<Message> = app
        .ai
        .turns
        .iter()
        .skip(skip)
        .filter(|t| t.role != Who::Problem)
        .map(|t| match t.role {
            Who::User => Message::user(t.sent.clone()),
            _ => Message::assistant(t.sent.clone()),
        })
        .collect();
    let session_key = app.ai.session_keys.get(&cfg.provider).cloned();
    spawn(app, move || {
        let key = if cfg.provider.needs_key() {
            session_key.or_else(|| codigo_ai::secret::load(cfg.provider))
        } else {
            None
        };
        let system = match gathered {
            Some(mut g) => {
                for (rel, abs) in g.from_disk {
                    if let Ok(text) = std::fs::read_to_string(&abs) {
                        g.context.attached.push((rel, text));
                    }
                }
                let ctx = g.context.render();
                if ctx.is_empty() {
                    prompts::SYSTEM.to_owned()
                } else {
                    format!("{}\n\n{ctx}", prompts::SYSTEM)
                }
            }
            None => prompts::SYSTEM.to_owned(),
        };
        let result = if cfg.provider.needs_key() && key.is_none() {
            Err(tr("Add your API key in the assistant settings first.").to_owned())
        } else {
            codigo_ai::complete(&cfg, key.as_deref(), &system, &messages).map_err(|e| e.to_string())
        };
        AiMsg::Reply {
            serial,
            target,
            result,
        }
    });
}

pub fn on_msg(app: &mut App, msg: AiMsg) {
    match msg {
        AiMsg::Reply {
            serial,
            target,
            result,
        } => {
            if serial != app.ai.serial {
                return;
            }
            app.ai.busy = false;
            app.ai.scroll_down = true;
            let (text, role) = match result {
                Ok(t) => (t, Who::Assistant),
                Err(e) => (e, Who::Problem),
            };
            let proposals = if role == Who::Assistant {
                edits::parse(&text)
                    .into_iter()
                    .map(|edit| Proposal {
                        edit,
                        stand: Stand::Pending,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let shown = if proposals.is_empty() {
                text.clone()
            } else {
                edits::strip(&text)
            };
            let uid = app.ai.fresh_uid();
            app.ai.turns.push(Turn {
                uid,
                edits: proposals,
                role,
                shown,
                sent: text,
                target,
            });
        }
        AiMsg::HasKey(provider, has) => {
            if let Overlay::AiSettings(s) = &mut app.overlay {
                if s.provider == provider {
                    s.has_key = Some(has);
                }
            }
        }
        AiMsg::KeyStored(provider, key, result) => {
            if let Overlay::AiSettings(s) = &mut app.overlay {
                s.key_input.clear();
                s.has_key = Some(true);
            }
            match result {
                Ok(()) => app.toast(tr("Key saved in the system keyring.")),
                Err(_) => {
                    app.ai.session_keys.insert(provider, key);
                    app.toast(tr(
                        "No keyring is available: the key is kept until Código closes.",
                    ));
                }
            }
        }
        AiMsg::Created {
            uid,
            index,
            path,
            result,
        } => {
            let stand = match result {
                Ok(()) => {
                    app.open_path(&path, None, true);
                    Stand::Applied
                }
                Err(e) => Stand::Failed(e),
            };
            if let Some(p) = app
                .ai
                .turns
                .iter_mut()
                .find(|t| t.uid == uid)
                .and_then(|t| t.edits.get_mut(index))
            {
                p.stand = stand;
            }
        }
        AiMsg::KeyRemoved(provider, result) => {
            app.ai.session_keys.remove(&provider);
            if let Overlay::AiSettings(s) = &mut app.overlay {
                s.has_key = Some(false);
            }
            match result {
                Ok(()) => app.toast(tr("Key removed.")),
                Err(e) => app.toast(e),
            }
        }
    }
}

/// Put the answer's code where the code it was asked about was.
fn apply(app: &mut App, turn: usize) {
    let Some((code, target)) = app
        .ai
        .turns
        .get(turn)
        .and_then(|t| Some((prompts::first_code_block(&t.sent)?, t.target.clone()?)))
    else {
        return;
    };
    let Some(tab) = app.tabs.iter_mut().find(|t| t.doc.id() == target.doc) else {
        app.toast(tr("That file is closed."));
        return;
    };
    if tab.doc.buffer().revision() != target.revision {
        app.toast(tr(
            "The file changed since you asked. Ask again to get an up-to-date answer.",
        ));
        return;
    }
    let old = tab.doc.buffer().slice(target.range.clone());
    let mut code = code;
    if old.ends_with('\n') && !code.ends_with('\n') {
        code.push('\n');
    }
    let mut done = false;
    tab.doc.apply(
        |b| match b.apply_edits(vec![Edit::new(target.range.clone(), code)]) {
            Some(changes) => {
                done = true;
                changes
            }
            None => Vec::new(),
        },
    );
    // The answer was applied against this text: it cannot be applied twice.
    if let Some(t) = app.ai.turns.get_mut(turn) {
        t.target = None;
    }
    app.toast(if done {
        tr("Applied. Undo brings the old code back.")
    } else {
        tr("The change could not be applied.")
    });
}

/// Put one proposed change into the project. Edits go through the open document, so Undo takes
/// them back and nothing is saved until the person saves; a new file is written off the UI thread.
fn apply_proposal(app: &mut App, turn: usize, index: usize) {
    let Some((uid, edit)) = app
        .ai
        .turns
        .get(turn)
        .and_then(|t| Some((t.uid, t.edits.get(index)?.edit.clone())))
    else {
        return;
    };
    let result = place(app, uid, index, &edit);
    if let Some(stand) = result {
        if let Some(p) = app
            .ai
            .turns
            .get_mut(turn)
            .and_then(|t| t.edits.get_mut(index))
        {
            p.stand = stand;
        }
    }
}

/// `None` while a write is on its way; otherwise how the change ended.
fn place(app: &mut App, uid: u64, index: usize, edit: &FileEdit) -> Option<Stand> {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else {
        return Some(Stand::Failed(tr("Open a folder first.").to_owned()));
    };
    let Some(rel) = edits::safe_relative(&edit.path) else {
        return Some(Stand::Failed(
            tr("That path is outside the project.").to_owned(),
        ));
    };
    let abs = root.join(&rel);
    match &edit.change {
        Change::Create { content } => {
            if abs.exists() {
                return Some(Stand::Failed(tr("That file already exists.").to_owned()));
            }
            let content = content.clone();
            let path = abs.clone();
            spawn(app, move || {
                let result = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, content))
                    .map_err(|e| e.to_string());
                AiMsg::Created {
                    uid,
                    index,
                    path,
                    result,
                }
            });
            None
        }
        Change::Replace { search, replace } => {
            Some(replace_in_open_file(app, &abs, search, replace))
        }
    }
}

fn replace_in_open_file(
    app: &mut App,
    abs: &std::path::Path,
    search: &str,
    replace: &str,
) -> Stand {
    if !abs.is_file() {
        return Stand::Failed(tr("That file does not exist.").to_owned());
    }
    if !app.tabs.iter().any(|t| t.doc.path() == Some(abs)) {
        app.open_path(abs, None, false);
    }
    let Some(i) = app.tabs.iter().position(|t| t.doc.path() == Some(abs)) else {
        return Stand::Failed(tr("The file is still opening. Try again in a moment.").to_owned());
    };
    let Some(tab) = app.tabs.get_mut(i) else {
        return Stand::Failed(tr("That file is closed.").to_owned());
    };
    let text = tab.doc.buffer().slice(0..tab.doc.buffer().len_chars());
    let bytes = match edits::locate(&text, search) {
        Ok(r) => r,
        Err(Locate::Missing) => {
            return Stand::Failed(
                tr("The text to replace is not in the file any more. Ask again.").to_owned(),
            )
        }
        Err(Locate::Ambiguous(n)) => {
            return Stand::Failed(trf(
                "The text to replace is in the file {} times, so it is not clear which one.",
                &[&n.to_string()],
            ))
        }
        Err(Locate::Empty) => {
            return Stand::Failed(tr("The change has no text to replace.").to_owned())
        }
    };
    let (Some(before), Some(inside)) = (text.get(..bytes.start), text.get(bytes.clone())) else {
        return Stand::Failed(tr("The change could not be applied.").to_owned());
    };
    let start = before.chars().count();
    let range = start..start + inside.chars().count();
    let mut done = false;
    tab.doc.apply(
        |b| match b.apply_edits(vec![Edit::new(range.clone(), replace.to_owned())]) {
            Some(changes) => {
                done = true;
                changes
            }
            None => Vec::new(),
        },
    );
    if !done {
        return Stand::Failed(tr("The change could not be applied.").to_owned());
    }
    let line = tab.doc.buffer().line_of(start);
    app.open_path(abs, Some(line), true);
    Stand::Applied
}

#[derive(Clone, Copy)]
enum CardAct {
    Apply,
    Reject,
}

/// The most lines of a diff drawn in a card before it scrolls.
const CARD_LINES: usize = 18;

/// One proposed change as a card: the file, the lines taken out and put in, and its buttons.
fn proposal_card(ui: &mut Ui, look: &Look, proposal: &Proposal) -> Option<CardAct> {
    let p = look.palette;
    let mut act = None;
    ui.add_space(4.0);
    egui::Frame::NONE
        .fill(color(p.surface_raised))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let (what, removed, added) = match &proposal.edit.change {
                Change::Replace { search, replace } => {
                    let (r, a) = edits::diff_lines(search, replace);
                    (tr("Change"), r, a)
                }
                Change::Create { content } => {
                    let (_, a) = edits::diff_lines("", content);
                    (tr("New file"), Vec::new(), a)
                }
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(what)
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
                ui.label(
                    RichText::new(&proposal.edit.path)
                        .font(FontId::monospace(font_size::SMALL))
                        .strong()
                        .color(color(p.text_primary)),
                );
            });
            ScrollArea::vertical()
                .id_salt(("ai-diff", &proposal.edit.path, removed.len(), added.len()))
                .max_height(CARD_LINES as f32 * 15.0)
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    for (mark, lines, ink) in [
                        ("−", &removed, p.status_danger_fg),
                        ("+", &added, p.status_success_fg),
                    ] {
                        for line in lines.iter().take(200) {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{mark} {line}"))
                                        .font(FontId::monospace(font_size::SMALL))
                                        .color(color(ink)),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    }
                });
            ui.add_space(2.0);
            match &proposal.stand {
                Stand::Pending => {
                    ui.horizontal(|ui| {
                        if cr::primary_button(ui, look, tr("Apply"), true).clicked() {
                            act = Some(CardAct::Apply);
                        }
                        if cr::secondary_button(ui, look, tr("Reject"), true).clicked() {
                            act = Some(CardAct::Reject);
                        }
                    });
                }
                Stand::Applied => {
                    ui.label(
                        RichText::new(tr("Applied. Undo brings the old code back."))
                            .size(font_size::SMALL)
                            .color(color(p.status_success_fg)),
                    );
                }
                Stand::Rejected => {
                    ui.label(
                        RichText::new(tr("Rejected"))
                            .size(font_size::SMALL)
                            .color(color(p.text_tertiary)),
                    );
                }
                Stand::Failed(why) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(why)
                                .size(font_size::SMALL)
                                .color(color(p.status_danger_fg)),
                        )
                        .wrap(),
                    );
                }
            }
        });
    act
}

pub struct Outcome {}

pub fn panel(ui: &mut Ui, app: &mut App, look: &Look) -> Outcome {
    let p = look.palette;
    let cfg = config(app);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} · {}", cfg.provider.name(), {
                let m = cfg.model.trim();
                if m.is_empty() {
                    "—"
                } else {
                    m
                }
            }))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        crate::widgets::trailing(ui, |ui| {
            if crate::widgets::small_icon_button(
                ui,
                look,
                "settings",
                tr("Assistant settings"),
                false,
            )
            .clicked()
            {
                open_settings(app);
            }
            if !app.ai.turns.is_empty()
                && crate::widgets::small_icon_button(
                    ui,
                    look,
                    "trash",
                    tr("Clear the conversation"),
                    false,
                )
                .clicked()
            {
                app.ai.turns.clear();
                app.ai.busy = false;
                app.ai.serial += 1;
            }
        });
    });
    ui.add_space(4.0);
    let input_height = 170.0;
    let height = (ui.available_height() - input_height).max(80.0);
    let mut apply_turn: Option<usize> = None;
    let mut copy: Option<String> = None;
    let mut card_act: Option<(usize, usize, CardAct)> = None;
    ScrollArea::vertical()
        .id_salt("ai-turns")
        .max_height(height)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if app.ai.turns.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(tr(
                            "Ask about your code, or use the Assistant menu on a selection. It can see the open file and propose changes you accept or reject. Mention a file with @name. Nothing is sent until you ask.",
                        ))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                    )
                    .wrap(),
                );
            }
            for (i, turn) in app.ai.turns.iter().enumerate() {
                ui.add_space(6.0);
                let (who, tint) = match turn.role {
                    Who::User => (tr("You"), color(p.text_secondary)),
                    Who::Assistant => (tr("Assistant"), color(p.accent_text)),
                    Who::Problem => (tr("Problem"), color(p.status_danger_fg)),
                };
                ui.label(RichText::new(who).strong().size(font_size::SMALL).color(tint));
                ui.add(
                    egui::Label::new(RichText::new(&turn.shown).color(color(p.text_primary)))
                        .wrap(),
                );
                if turn.role == Who::Assistant {
                    for (n, proposal) in turn.edits.iter().enumerate() {
                        if let Some(act) = proposal_card(ui, look, proposal) {
                            card_act = Some((i, n, act));
                        }
                    }
                    let pending = turn
                        .edits
                        .iter()
                        .filter(|p| p.stand == Stand::Pending)
                        .count();
                    if pending > 1
                        && ui
                            .small_button(trf("Apply all {} changes", &[&pending.to_string()]))
                            .clicked()
                    {
                        card_act = Some((i, usize::MAX, CardAct::Apply));
                    }
                    if turn.edits.is_empty() {
                        if let Some(code) = prompts::first_code_block(&turn.sent) {
                            ui.horizontal(|ui| {
                                if let Some(t) = &turn.target {
                                    if ui.small_button(trf("Apply to {}", &[&t.file])).clicked() {
                                        apply_turn = Some(i);
                                    }
                                }
                                if ui.small_button(tr("Copy the code")).clicked() {
                                    copy = Some(code);
                                }
                            });
                        }
                    }
                }
            }
            if app.ai.busy {
                ui.add_space(6.0);
                ui.label(RichText::new(tr("Thinking…")).color(color(p.text_secondary)));
            }
        });
    if let Some(i) = apply_turn {
        apply(app, i);
    }
    if let Some((turn, index, act)) = card_act {
        let indexes: Vec<usize> = if index == usize::MAX {
            app.ai.turns.get(turn).map_or_else(Vec::new, |t| {
                (0..t.edits.len())
                    .filter(|n| t.edits.get(*n).is_some_and(|p| p.stand == Stand::Pending))
                    .collect()
            })
        } else {
            vec![index]
        };
        for n in indexes {
            match act {
                CardAct::Apply => apply_proposal(app, turn, n),
                CardAct::Reject => {
                    if let Some(p) = app.ai.turns.get_mut(turn).and_then(|t| t.edits.get_mut(n)) {
                        p.stand = Stand::Rejected;
                    }
                }
            }
        }
    }
    if let Some(code) = copy {
        app.egui_ctx().copy_text(code);
        app.toast(tr("Copied."));
    }
    ui.add_space(6.0);
    let mut send_now = false;
    let response = cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.ai.input)
                    .frame(egui::Frame::NONE)
                    .hint_text(tr("Ask the assistant…"))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(font_size::BODY)),
            )
        })
        .inner;
    if app.ai.focus {
        response.request_focus();
        app.ai.focus = false;
    }
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command)
    {
        send_now = true;
    }
    if !app.settings.ai_context {
        ui.checkbox(&mut app.ai.with_selection, tr("Include the selected code"));
    }
    ui.label(
        RichText::new(context_line(app))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
    );
    ui.horizontal(|ui| {
        let ready = !app.ai.input.trim().is_empty() && !app.ai.busy;
        if cr::primary_button(ui, look, tr("Send"), ready).clicked() {
            send_now = true;
        }
    });
    if cfg.provider != Provider::Ollama {
        ui.label(
            RichText::new(if app.settings.ai_context {
                trf(
                    "What is listed above goes to {} with your question.",
                    &[cfg.provider.name()],
                )
            } else {
                trf("Your question goes to {}.", &[cfg.provider.name()])
            })
            .size(font_size::SMALL)
            .color(color(p.text_tertiary)),
        );
    }
    if send_now {
        ask_from_input(app);
    }
    Outcome {}
}

// --- Settings ---------------------------------------------------------------------------------

pub struct AiSettings {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub key_input: String,
    /// Whether the keyring holds a key for the provider; `None` while it is being asked.
    pub has_key: Option<bool>,
}

pub fn open_settings(app: &mut App) {
    let cfg = config(app);
    app.overlay = Overlay::AiSettings(AiSettings {
        provider: cfg.provider,
        model: app.settings.ai_model.clone(),
        base_url: app.settings.ai_base_url.clone(),
        key_input: String::new(),
        has_key: None,
    });
    ask_has_key(app, cfg.provider);
}

fn ask_has_key(app: &App, provider: Provider) {
    spawn(app, move || {
        AiMsg::HasKey(provider, codigo_ai::secret::load(provider).is_some())
    });
}

pub fn settings_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut state: AiSettings,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut save_key = false;
    let mut remove_key = false;
    let mut switched: Option<Provider> = None;
    let dismissed = modal(ctx, look, "ai-settings", 560.0, true, |ui| {
        closed = heading(ui, look, tr("Assistant settings"), "ai-settings");
        ui.label(
            RichText::new(tr("Provider"))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        ui.horizontal_wrapped(|ui| {
            for provider in Provider::ALL {
                if ui
                    .selectable_label(state.provider == provider, provider.name())
                    .clicked()
                    && state.provider != provider
                {
                    switched = Some(provider);
                }
            }
        });
        ui.add_space(8.0);
        field(
            ui,
            look,
            tr("Model"),
            &mut state.model,
            state.provider.default_model(),
        );
        ui.add_space(6.0);
        field(
            ui,
            look,
            tr("Address (leave empty for the usual one)"),
            &mut state.base_url,
            state.provider.default_base(),
        );
        if state.provider.needs_key() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(match state.has_key {
                    Some(true) => tr("An API key is saved."),
                    Some(false) => tr("No API key is saved yet."),
                    None => tr("Looking for a saved key…"),
                })
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
            );
            cr::field_frame(look)
                .inner_margin(egui::Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.key_input)
                            .password(true)
                            .frame(egui::Frame::NONE)
                            .hint_text(tr("Paste an API key"))
                            .align(Align2::LEFT_CENTER)
                            .desired_width(f32::INFINITY),
                    )
                });
            ui.horizontal(|ui| {
                let ok = codigo_ai::secret::clean(&state.key_input).is_some();
                if cr::primary_button(ui, look, tr("Save key"), ok).clicked() {
                    save_key = true;
                }
                if cr::secondary_button(ui, look, tr("Remove key"), state.has_key == Some(true))
                    .clicked()
                {
                    remove_key = true;
                }
            });
            ui.label(
                RichText::new(tr(
                    "The key is kept in the system keyring and is sent only to the provider.",
                ))
                .size(font_size::SMALL)
                .color(color(p.text_tertiary)),
            );
        } else if state.provider == Provider::Ollama {
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr("Ollama runs on this computer: nothing leaves it."))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
        }
    });
    if let Some(provider) = switched {
        state.provider = provider;
        state.model.clear();
        state.base_url.clear();
        state.key_input.clear();
        state.has_key = None;
        ask_has_key(app, provider);
    }
    if save_key {
        if let Some(key) = codigo_ai::secret::clean(&state.key_input) {
            let provider = state.provider;
            spawn(app, move || {
                let result = codigo_ai::secret::store(provider, &key).map_err(|e| e.to_string());
                AiMsg::KeyStored(provider, key, result)
            });
        }
    }
    if remove_key {
        let provider = state.provider;
        spawn(app, move || {
            AiMsg::KeyRemoved(
                provider,
                codigo_ai::secret::delete(provider).map_err(|e| e.to_string()),
            )
        });
    }
    // Choices apply as they are made.
    let changed = app.settings.ai_provider != state.provider.id()
        || app.settings.ai_model != state.model.trim()
        || app.settings.ai_base_url != state.base_url.trim();
    if changed {
        app.settings.ai_provider = state.provider.id().to_owned();
        app.settings.ai_model = state.model.trim().to_owned();
        app.settings.ai_base_url = state.base_url.trim().to_owned();
        app.settings.sanitize();
        app.save_settings();
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::AiSettings(state), false)
}

fn field(ui: &mut Ui, look: &Look, label: &str, value: &mut String, hint: &str) {
    let p = look.palette;
    ui.label(
        RichText::new(label)
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
    );
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            )
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_provider_follows_the_setting() {
        assert_eq!(Provider::from_id("ollama"), Provider::Ollama);
        assert_eq!(Provider::from_id("nonsense"), Provider::Anthropic);
    }
}
