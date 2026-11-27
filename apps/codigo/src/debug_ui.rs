// SPDX-License-Identifier: MIT OR Apache-2.0
//! Debugging in the window: the state of a session, the sidebar panel that shows it and the
//! commands that drive it.
//!
//! The session itself (`codigo-dap`) runs on its own thread. This module keeps what the window
//! needs between frames (breakpoints, the stack it stopped on, the variables it opened) and turns
//! the session's updates into [`Effect`]s the app applies. [`DebugPanel::apply`] has no UI in it,
//! so the state machine is tested without a window.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codigo_dap::{
    adapters_for, prepare, Command, Debugger, Install, LaunchSpec, Missing, Prepare, ScopeVars,
    StopInfo, Update, Variable,
};
use egui::{CornerRadius, FontId, RichText, ScrollArea, Sense, TextEdit, Ui};
use nodalia_theme::creator::Look;
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg, Panel};
use crate::i18n::{tr, trf};

const MAX_OUTPUT: usize = 400;
const MAX_EVALS: usize = 20;
/// The most the call stack takes of the panel before it scrolls on its own.
const STACK_HEIGHT: f32 = 156.0;
/// Scopes that are only noise in a panel this narrow.
const HIDDEN_SCOPES: [&str; 2] = ["Registers", "Globals"];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    #[default]
    Idle,
    /// Building the program and looking for a debugger.
    Preparing,
    /// The program runs.
    Running,
    /// The program is stopped and its state is on screen.
    Stopped,
}

/// What the app has to do after an update.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Open this file at this 0-based line.
    Show(PathBuf, usize),
    Toast(String),
    /// Bring the debug panel up.
    Reveal,
}

#[derive(Default)]
pub struct DebugPanel {
    pub phase: Phase,
    session: Option<Debugger>,
    /// Breakpoints by file, as 1-based lines.
    pub breakpoints: BTreeMap<PathBuf, BTreeSet<usize>>,
    pub stop: Option<StopInfo>,
    children: HashMap<i64, Vec<Variable>>,
    open: BTreeSet<i64>,
    pub output: VecDeque<String>,
    pub eval: String,
    pub evals: Vec<(String, Result<String, String>)>,
    /// Tells a late answer of a start the person already cancelled from the current one.
    pub serial: u64,
    /// Why the last session could not start or ended badly.
    pub notice: Option<String>,
    /// The language has no debugger on this computer, and how to get one.
    pub missing: Option<Missing>,
}

/// Why a session could not be prepared.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The build or the project said so, in its own words.
    Text(String),
    /// No debugger is installed.
    Missing(Missing),
}

/// What to tell somebody who has no debugger yet, in the interface's language.
pub fn missing_hint(install: &Install) -> &'static str {
    match install {
        Install::XcodeTools => tr(
            "Install Apple's developer tools: they bring the debugger. Then press F6 again.",
        ),
        Install::XcodeToolsOld => tr(
            "The developer tools here are too old to carry the debugger. Update them to Xcode 16 or newer, or install LLVM with Homebrew.",
        ),
        Install::Command(_) => tr("Install the debugger from a terminal, then press F6 again."),
        Install::Python => tr("Install Python, then its debugger, and press F6 again."),
        Install::Unsupported => tr("Código can debug Rust, C, C++, Python and Go so far."),
    }
}

impl DebugPanel {
    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The 0-based lines of a file that carry a breakpoint, sorted.
    pub fn lines_of(&self, path: &Path) -> Vec<usize> {
        self.breakpoints
            .get(path)
            .map(|s| s.iter().map(|l| l.saturating_sub(1)).collect())
            .unwrap_or_default()
    }

    /// The 0-based line the program is stopped on, when that is in `path`.
    pub fn exec_line(&self, path: &Path) -> Option<usize> {
        let stop = self.stop.as_ref()?;
        let frame = stop.frames.get(stop.selected)?;
        (frame.path.as_deref() == Some(path)).then(|| frame.line.saturating_sub(1))
    }

    /// Set or clear the breakpoint on a 0-based line, and tell a running session.
    pub fn toggle(&mut self, path: &Path, line0: usize) {
        let line = line0 + 1;
        let lines = self.breakpoints.entry(path.to_path_buf()).or_default();
        if !lines.remove(&line) {
            lines.insert(line);
        }
        let now: Vec<usize> = lines.iter().copied().collect();
        if now.is_empty() {
            self.breakpoints.remove(path);
        }
        if let Some(s) = &self.session {
            s.send(Command::SetBreakpoints(path.to_path_buf(), now));
        }
    }

    pub fn remove_breakpoint(&mut self, path: &Path, line: usize) {
        self.toggle(path, line.saturating_sub(1));
    }

    fn send(&self, command: Command) {
        if let Some(s) = &self.session {
            s.send(command);
        }
    }

    /// Resume or step, when the program is stopped.
    pub fn resume(&mut self, command: Command) {
        if self.phase == Phase::Stopped {
            self.send(command);
        }
    }

    pub fn stop_session(&mut self) {
        self.send(Command::Stop);
        self.session = None;
        self.reset();
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.stop = None;
        self.children.clear();
        self.open.clear();
        self.serial += 1;
    }

    pub fn select_frame(&mut self, index: usize) {
        self.send(Command::SelectFrame(index));
    }

    /// Show or hide the children of a variable, asking the adapter the first time.
    pub fn toggle_variable(&mut self, reference: i64) {
        if reference == 0 {
            return;
        }
        if !self.open.remove(&reference) {
            self.open.insert(reference);
            if !self.children.contains_key(&reference) {
                self.send(Command::Expand(reference));
            }
        }
    }

    pub fn evaluate(&mut self) {
        let expression = self.eval.trim().to_string();
        if !expression.is_empty() && self.phase == Phase::Stopped {
            self.send(Command::Evaluate(expression));
            self.eval.clear();
        }
    }

    fn push_output(&mut self, text: &str) {
        for line in text.lines() {
            self.output.push_back(line.to_string());
            if self.output.len() > MAX_OUTPUT {
                self.output.pop_front();
            }
        }
    }

    /// Fold one update from the session in, and say what the window should do about it.
    pub fn apply(&mut self, update: Update) -> Vec<Effect> {
        let mut effects = Vec::new();
        match update {
            Update::Started | Update::Running => {
                self.phase = Phase::Running;
                self.stop = None;
                self.children.clear();
                self.open.clear();
            }
            Update::Stopped(mut info) => {
                normalise(&mut info);
                self.phase = Phase::Stopped;
                self.children.clear();
                self.open.clear();
                effects.push(Effect::Reveal);
                effects.extend(show_frame(&info));
                self.stop = Some(info);
            }
            Update::Frame { selected, scopes } => {
                if let Some(stop) = self.stop.as_mut() {
                    stop.selected = selected.min(stop.frames.len().saturating_sub(1));
                    stop.scopes = scopes;
                    self.children.clear();
                    self.open.clear();
                    effects.extend(show_frame(stop));
                }
            }
            Update::Children { reference, vars } => {
                self.children.insert(reference, vars);
            }
            Update::Evaluated { expression, result } => {
                self.evals.push((expression, result));
                if self.evals.len() > MAX_EVALS {
                    drop(self.evals.remove(0));
                }
            }
            Update::Breakpoints { .. } => {}
            Update::Output { category, text } => {
                if category != "telemetry" {
                    self.push_output(&text);
                }
            }
            Update::Ended { exit_code, error } => {
                self.session = None;
                self.reset();
                match (error, exit_code) {
                    (Some(e), _) => {
                        self.notice = Some(e.clone());
                        effects.push(Effect::Toast(e));
                    }
                    (None, Some(code)) => effects.push(Effect::Toast(trf(
                        "The program exited with code {}.",
                        &[&code.to_string()],
                    ))),
                    (None, None) => effects.push(Effect::Toast(
                        tr("The debugging session ended.").to_string(),
                    )),
                }
            }
        }
        effects
    }

    /// The session's pending updates, applied.
    pub fn poll(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        while let Some(update) = self.session.as_ref().and_then(|s| s.poll()) {
            effects.extend(self.apply(update));
        }
        effects
    }
}

/// Frame paths come from the adapter in whatever form it likes; open tabs use canonical ones.
fn normalise(info: &mut StopInfo) {
    for f in &mut info.frames {
        if let Some(p) = f.path.take() {
            f.path = Some(std::fs::canonicalize(&p).unwrap_or(p));
        }
    }
}

fn show_frame(info: &StopInfo) -> Option<Effect> {
    let frame = info.frames.get(info.selected)?;
    let path = frame.path.clone().filter(|p| p.is_file())?;
    Some(Effect::Show(path, frame.line.saturating_sub(1)))
}

// --- Driving it from the app -----------------------------------------------------------------

/// Build the program and start debugging it, off the UI thread.
pub fn start(app: &mut App) {
    if app.debug.active() {
        app.toast(tr("A debugging session is already running."));
        return;
    }
    let Some(root) = crate::setup_ui::work_root(app) else {
        app.toast(tr("Open a folder first."));
        return;
    };
    if !app.save_all() {
        return;
    }
    let active: Option<PathBuf> = app
        .tabs
        .get(app.active)
        .and_then(|t| t.doc.path().map(Path::to_path_buf));
    let breakpoints: BTreeMap<PathBuf, Vec<usize>> = app
        .debug
        .breakpoints
        .iter()
        .map(|(p, l)| (p.clone(), l.iter().copied().collect()))
        .collect();
    app.debug.phase = Phase::Preparing;
    app.debug.notice = None;
    app.debug.missing = None;
    app.debug.output.clear();
    app.debug.evals.clear();
    app.debug.serial += 1;
    let serial = app.debug.serial;
    app.sidebar_visible = true;
    app.panel = Panel::Debug;
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let spawned = std::thread::Builder::new()
        .name("codigo-debug-prepare".into())
        .spawn(move || {
            let msg = match prepare(&root, active.as_deref()) {
                Err(Prepare::Unknown(note)) => Msg::DebugAsk(serial, note),
                Err(Prepare::Failed(e)) => Msg::DebugSpec(serial, Err(Failure::Text(e))),
                Ok(target) => {
                    let spec = adapters_for(target.language)
                        .map_err(Failure::Missing)
                        .and_then(|found| {
                            found.into_iter().next().ok_or_else(|| {
                                Failure::Missing(Missing {
                                    language: target.language.to_string(),
                                    install: Install::Unsupported,
                                })
                            })
                        })
                        .map(|adapter| LaunchSpec {
                            adapter,
                            program: target.program,
                            args: target.args,
                            cwd: target.cwd,
                            breakpoints,
                        });
                    Msg::DebugSpec(serial, spec)
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        app.debug.reset();
        app.toast(tr("Could not start the debugger."));
    }
}

/// The program was built and a debugger found: start the session.
pub fn launch(app: &mut App, serial: u64, spec: Result<LaunchSpec, Failure>) {
    if serial != app.debug.serial || app.debug.phase != Phase::Preparing {
        return;
    }
    match spec {
        Ok(spec) => {
            let ctx = app.egui_ctx().clone();
            let waker: codigo_dap::session::Waker = Arc::new(move || ctx.request_repaint());
            app.debug.session = Some(Debugger::start(spec, Some(waker)));
            app.debug.phase = Phase::Running;
        }
        Err(Failure::Text(e)) => {
            app.debug.reset();
            app.debug.notice = Some(e.clone());
            app.toast(e);
        }
        Err(Failure::Missing(m)) => {
            app.debug.reset();
            app.toast(trf("No debugger found for {}.", &[&m.language]));
            app.debug.missing = Some(m);
        }
    }
}

/// Nothing recognisable to debug: ask which program, and start once it is saved.
pub fn ask(app: &mut App, serial: u64, note: String) {
    if serial != app.debug.serial || app.debug.phase != Phase::Preparing {
        return;
    }
    app.debug.reset();
    let Some(root) = crate::setup_ui::work_root(app) else {
        return;
    };
    app.overlay =
        crate::overlays::Overlay::DebugSetup(crate::setup_ui::DebugSetup::new(&root, note));
}

pub fn stop(app: &mut App) {
    app.debug.stop_session();
}

/// Called once a frame: apply what the session sent.
pub fn poll(app: &mut App) {
    for effect in app.debug.poll() {
        match effect {
            Effect::Show(path, line) => {
                app.open_path(&path, Some(line), false);
            }
            Effect::Toast(text) => app.toast(text),
            Effect::Reveal => {
                app.sidebar_visible = true;
                app.panel = Panel::Debug;
            }
        }
    }
}

/// Toggle a breakpoint on the caret's line.
pub fn toggle_at_caret(app: &mut App) {
    let Some(tab) = app.tabs.get(app.active) else {
        return;
    };
    let Some(path) = tab.doc.path().map(Path::to_path_buf) else {
        app.toast(tr("Save the file first to set a breakpoint in it."));
        return;
    };
    let buffer = tab.doc.buffer();
    let line = buffer.line_of(buffer.primary().head);
    app.debug.toggle(&path, line);
}

// --- The panel -------------------------------------------------------------------------------

/// What the panel asks for.
#[derive(Default)]
pub struct Outcome {
    pub open: Option<(PathBuf, usize)>,
    /// "Try again" after installing a debugger.
    pub retry: bool,
}

/// What a missing debugger looks like: what is wrong, how to fix it, and the buttons to do it.
/// True when the person asked to try again.
fn missing_view(ui: &mut Ui, look: &Look, m: &Missing) -> bool {
    use nodalia_theme::creator as cr;
    let p = look.palette;
    let mut retry = false;
    ui.add_space(6.0);
    ui.label(
        RichText::new(trf("No debugger found for {}.", &[&m.language]))
            .strong()
            .color(color(p.status_danger_fg)),
    );
    ui.label(
        RichText::new(missing_hint(&m.install))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
    );
    if let Some(command) = m.install.command() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(&command)
                    .font(FontId::monospace(font_size::SMALL))
                    .color(color(p.text_primary)),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if cr::secondary_button(ui, look, tr("Copy the command"), true).clicked() {
                ui.ctx().copy_text(command.clone());
            }
            if m.install == Install::XcodeTools
                && cr::primary_button(ui, look, tr("Install the developer tools"), true).clicked()
            {
                install_xcode_tools();
            }
        });
    }
    ui.add_space(2.0);
    if cr::secondary_button(ui, look, tr("Try again"), true).clicked() {
        retry = true;
    }
    retry
}

/// Open Apple's installer for the command line tools. It is a window of its own and the call
/// returns at once; waiting for it happens off the UI thread so no process is left behind.
fn install_xcode_tools() {
    let _ = std::thread::Builder::new()
        .name("codigo-xcode-select".into())
        .spawn(|| {
            let _ = std::process::Command::new("xcode-select")
                .arg("--install")
                .status();
        });
}

pub fn panel(ui: &mut Ui, app: &mut App, look: &Look) -> Outcome {
    let p = look.palette;
    let mut outcome = Outcome::default();
    let root = app.project.as_ref().map(|p| p.root.clone());
    let d = &mut app.debug;
    match d.phase {
        Phase::Idle => {
            ui.label(
                RichText::new(tr(
                    "Press F6 or the bug button to run the project under the debugger. Click the margin of the editor to set breakpoints.",
                ))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
            );
            if let Some(n) = &d.notice {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(n)
                        .size(font_size::SMALL)
                        .color(color(p.status_danger_fg)),
                );
            }
            if let Some(m) = &d.missing {
                outcome.retry = missing_view(ui, look, m);
            }
        }
        Phase::Preparing => {
            ui.label(
                RichText::new(tr("Building and starting the debugger…"))
                    .color(color(p.text_secondary)),
            );
        }
        Phase::Running => {
            ui.label(RichText::new(tr("Running…")).color(color(p.status_success_fg)));
        }
        Phase::Stopped => {
            let reason = d
                .stop
                .as_ref()
                .map(|s| s.reason.clone())
                .unwrap_or_default();
            ui.label(
                RichText::new(trf("Paused: {}", &[&reason]))
                    .strong()
                    .color(color(p.status_warning_fg)),
            );
        }
    }
    ui.add_space(6.0);
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("debug-panel")
        .show(ui, |ui| {
            if d.phase == Phase::Stopped {
                stack(ui, d, look, &mut outcome);
                variables(ui, d, look);
                watch(ui, d, look);
            }
            breakpoints(ui, d, look, root.as_deref(), &mut outcome);
            output(ui, d, look);
        });
    outcome
}

fn heading(ui: &mut Ui, look: &Look, text: impl Into<String>) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text)
            .strong()
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
}

fn stack(ui: &mut Ui, d: &mut DebugPanel, look: &Look, outcome: &mut Outcome) {
    let Some(stop) = d.stop.clone() else { return };
    heading(ui, look, tr("Call stack"));
    ScrollArea::vertical()
        .id_salt("debug-stack")
        .max_height(STACK_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| stack_rows(ui, d, look, &stop, outcome));
}

fn stack_rows(
    ui: &mut Ui,
    d: &mut DebugPanel,
    look: &Look,
    stop: &StopInfo,
    outcome: &mut Outcome,
) {
    let p = look.palette;
    for (i, f) in stop.frames.iter().enumerate() {
        let file = f
            .path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = if file.is_empty() {
            f.name.clone()
        } else {
            format!("{}  {}:{}", f.name, file, f.line)
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
        if i == stop.selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), color(p.control_bg));
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), color(p.control_bg_hover));
        }
        ui.painter().with_clip_rect(rect).text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(font_size::SMALL + 1.0),
            color(p.text_primary),
        );
        if response.clicked() {
            d.select_frame(i);
            if let Some(path) = f.path.clone() {
                outcome.open = Some((path, f.line.saturating_sub(1)));
            }
        }
    }
}

fn variables(ui: &mut Ui, d: &mut DebugPanel, look: &Look) {
    let Some(stop) = d.stop.clone() else { return };
    for scope in stop
        .scopes
        .iter()
        .filter(|s| !HIDDEN_SCOPES.contains(&s.name.as_str()))
    {
        heading(ui, look, scope.name.clone());
        scope_rows(ui, d, look, scope);
    }
}

fn scope_rows(ui: &mut Ui, d: &mut DebugPanel, look: &Look, scope: &ScopeVars) {
    for v in &scope.vars {
        variable_row(ui, d, look, v, 0);
    }
}

fn variable_row(ui: &mut Ui, d: &mut DebugPanel, look: &Look, v: &Variable, depth: usize) {
    let p = look.palette;
    let open = d.open.contains(&v.reference);
    let marker = if v.reference == 0 {
        "  "
    } else if open {
        "▾ "
    } else {
        "▸ "
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), color(p.control_bg_hover));
    }
    let indent = 4.0 + depth as f32 * 14.0;
    let painter = ui.painter().with_clip_rect(rect);
    let name = format!("{marker}{}", v.name);
    let font = FontId::monospace(font_size::SMALL + 1.0);
    let name_w = painter
        .layout_no_wrap(name.clone(), font.clone(), color(p.text_primary))
        .size()
        .x;
    painter.text(
        rect.left_center() + egui::vec2(indent, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        font.clone(),
        color(p.text_primary),
    );
    painter.text(
        rect.left_center() + egui::vec2(indent + name_w + 10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &v.value,
        font,
        color(p.text_secondary),
    );
    let response = if v.type_name.is_empty() {
        response
    } else {
        response.on_hover_text(&v.type_name)
    };
    if response.clicked() {
        d.toggle_variable(v.reference);
    }
    if open {
        if let Some(children) = d.children.get(&v.reference).cloned() {
            for c in &children {
                variable_row(ui, d, look, c, depth + 1);
            }
        }
    }
}

fn watch(ui: &mut Ui, d: &mut DebugPanel, look: &Look) {
    let p = look.palette;
    heading(ui, look, tr("Evaluate"));
    let response = ui.add(
        TextEdit::singleline(&mut d.eval)
            .hint_text(tr("Expression, then Enter"))
            .font(FontId::monospace(font_size::SMALL + 1.0))
            .desired_width(f32::INFINITY),
    );
    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        d.evaluate();
        response.request_focus();
    }
    for (expr, result) in d.evals.iter().rev().take(6) {
        let (text, ink) = match result {
            Ok(v) => (format!("{expr} = {v}"), p.text_primary),
            Err(e) => (format!("{expr}: {e}"), p.status_danger_fg),
        };
        ui.label(
            RichText::new(text)
                .font(FontId::monospace(font_size::SMALL))
                .color(color(ink)),
        );
    }
}

fn breakpoints(
    ui: &mut Ui,
    d: &mut DebugPanel,
    look: &Look,
    root: Option<&Path>,
    outcome: &mut Outcome,
) {
    let p = look.palette;
    heading(ui, look, tr("Breakpoints"));
    if d.breakpoints.is_empty() {
        ui.label(
            RichText::new(tr("None. Click a line number's margin, or press F9."))
                .size(font_size::SMALL)
                .color(color(p.text_tertiary)),
        );
        return;
    }
    let all: Vec<(PathBuf, usize)> = d
        .breakpoints
        .iter()
        .flat_map(|(path, lines)| lines.iter().map(|l| (path.clone(), *l)))
        .collect();
    let mut remove = None;
    for (path, line) in &all {
        let shown = root
            .and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 16.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 4.0, color(p.status_danger_fg));
            let label = ui.add(
                egui::Label::new(
                    RichText::new(format!("{shown}:{line}")).size(font_size::SMALL + 1.0),
                )
                .sense(Sense::click())
                .truncate(),
            );
            if label.clicked() {
                outcome.open = Some((path.clone(), line.saturating_sub(1)));
            }
            if ui
                .small_button("✕")
                .on_hover_text(tr("Remove breakpoint"))
                .clicked()
            {
                remove = Some((path.clone(), *line));
            }
        });
    }
    if let Some((path, line)) = remove {
        d.remove_breakpoint(&path, line);
    }
}

fn output(ui: &mut Ui, d: &DebugPanel, look: &Look) {
    if d.output.is_empty() {
        return;
    }
    heading(ui, look, tr("Output"));
    for line in d
        .output
        .iter()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        ui.label(
            RichText::new(line)
                .font(FontId::monospace(font_size::SMALL))
                .color(color(look.palette.text_secondary)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codigo_dap::Frame;

    fn stop_at(path: &Path, line: usize) -> StopInfo {
        StopInfo {
            reason: "breakpoint".into(),
            thread_id: 1,
            frames: vec![Frame {
                id: 1,
                name: "main".into(),
                path: Some(path.to_path_buf()),
                line,
                column: 1,
            }],
            selected: 0,
            scopes: Vec::new(),
        }
    }

    #[test]
    fn breakpoints_toggle_by_zero_based_line_and_store_one_based() {
        let mut d = DebugPanel::default();
        let f = PathBuf::from("/x/main.rs");
        d.toggle(&f, 4);
        assert_eq!(d.breakpoints[&f].iter().copied().collect::<Vec<_>>(), [5]);
        assert_eq!(d.lines_of(&f), [4]);
        d.toggle(&f, 4);
        assert!(d.breakpoints.is_empty());
        assert!(d.lines_of(&f).is_empty());
    }

    #[test]
    fn a_stop_shows_the_frame_and_marks_the_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = std::fs::canonicalize(dir.path())
            .expect("canonical")
            .join("a.rs");
        std::fs::write(&f, "fn main() {}\n").expect("write");
        let mut d = DebugPanel::default();
        let effects = d.apply(Update::Stopped(stop_at(&f, 7)));
        assert_eq!(d.phase, Phase::Stopped);
        assert_eq!(effects, vec![Effect::Reveal, Effect::Show(f.clone(), 6)]);
        assert_eq!(d.exec_line(&f), Some(6));
        assert_eq!(d.exec_line(Path::new("/other.rs")), None);
        d.apply(Update::Running);
        assert_eq!(d.phase, Phase::Running);
        assert_eq!(d.exec_line(&f), None);
    }

    #[test]
    fn a_stop_in_a_file_that_does_not_exist_opens_nothing() {
        let mut d = DebugPanel::default();
        let effects = d.apply(Update::Stopped(stop_at(Path::new("/no/such/file.rs"), 1)));
        assert_eq!(effects, vec![Effect::Reveal]);
    }

    #[test]
    fn ending_goes_back_to_idle_and_says_why() {
        let mut d = DebugPanel::default();
        d.apply(Update::Running);
        let e = d.apply(Update::Ended {
            exit_code: None,
            error: Some("adapter died".into()),
        });
        assert_eq!(d.phase, Phase::Idle);
        assert_eq!(d.notice.as_deref(), Some("adapter died"));
        assert_eq!(e, vec![Effect::Toast("adapter died".into())]);
    }

    #[test]
    fn output_is_capped_and_telemetry_is_dropped() {
        let mut d = DebugPanel::default();
        for i in 0..(MAX_OUTPUT + 50) {
            d.apply(Update::Output {
                category: "stdout".into(),
                text: format!("line {i}"),
            });
        }
        d.apply(Update::Output {
            category: "telemetry".into(),
            text: "secret".into(),
        });
        assert_eq!(d.output.len(), MAX_OUTPUT);
        assert!(!d.output.iter().any(|l| l == "secret"));
        assert_eq!(d.output.back().map(String::as_str), Some("line 449"));
    }

    #[test]
    fn stepping_is_ignored_unless_the_program_is_stopped() {
        let mut d = DebugPanel::default();
        d.resume(Command::Continue);
        d.toggle_variable(0);
        assert_eq!(d.phase, Phase::Idle);
    }
}
