// SPDX-License-Identifier: MIT OR Apache-2.0
//! Asking how to run and debug a project Código does not recognise, and remembering the answer
//! in the project's `.codigo` folder so it is only asked once.

use std::path::{Path, PathBuf};

use codigo_dap::Saved;
use codigo_engine::tasks::{self, TaskKind};
use egui::{Align, Align2, Layout, RichText};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::tr;
use crate::overlays::{heading, modal, Overlay, Shown};

/// "How do you run this project?"
pub struct RunSetup {
    pub kind: TaskKind,
    pub command: String,
    pub error: Option<String>,
    pub focus: bool,
}

impl RunSetup {
    pub fn new(kind: TaskKind) -> RunSetup {
        RunSetup {
            kind,
            command: String::new(),
            error: None,
            focus: true,
        }
    }
}

/// "What should the debugger start?"
pub struct DebugSetup {
    pub program: String,
    pub args: String,
    pub build: String,
    pub note: String,
    pub error: Option<String>,
    pub focus: bool,
}

impl DebugSetup {
    /// A form filled with what is already saved, if anything.
    pub fn new(root: &Path, note: String) -> DebugSetup {
        let saved = codigo_dap::saved(root);
        DebugSetup {
            program: saved
                .as_ref()
                .map(|s| s.program.clone())
                .unwrap_or_default(),
            args: saved.as_ref().map(|s| s.args.join(" ")).unwrap_or_default(),
            build: saved.and_then(|s| s.build).unwrap_or_default(),
            note,
            error: None,
            focus: true,
        }
    }
}

fn caption(ui: &mut egui::Ui, look: &Look, text: &str) {
    ui.label(
        RichText::new(text)
            .strong()
            .size(font_size::BODY)
            .color(color(look.palette.text_secondary)),
    );
}

/// A single-line field in the creator style. Returns its response.
fn field(ui: &mut egui::Ui, look: &Look, text: &mut String, hint: &str) -> egui::Response {
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::monospace(font_size::BODY)),
            )
        })
        .inner
}

fn kind_title(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Run => tr("How do you run this project?"),
        TaskKind::Build => tr("How do you build this project?"),
        TaskKind::Test => tr("How do you test this project?"),
        TaskKind::Check => tr("How do you check this project?"),
    }
}

/// The folder a task or a debug session works in: the open folder, or the folder of the file in
/// the editor when there is none.
pub fn work_root(app: &App) -> Option<PathBuf> {
    app.project.as_ref().map(|p| p.root.clone()).or_else(|| {
        app.active_doc()
            .and_then(|d| d.path())
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    })
}

pub fn run_setup(ctx: &egui::Context, app: &mut App, look: &Look, mut state: RunSetup) -> Shown {
    let mut save = false;
    let mut closed = false;
    let dismissed = modal(ctx, look, "run-setup", 540.0, false, |ui| {
        closed = heading(ui, look, kind_title(state.kind), "run-setup");
        ui.label(
            RichText::new(tr(
                "Código did not recognise this project. Type the command once; it is saved in the project's .codigo folder and used from now on.",
            ))
            .color(color(look.palette.text_secondary)),
        );
        ui.add_space(12.0);
        caption(ui, look, tr("Command"));
        let response = field(
            ui,
            look,
            &mut state.command,
            tr("npm run dev, python3 app.py, make run…"),
        );
        if state.focus {
            response.request_focus();
            state.focus = false;
        }
        if response.changed() {
            state.error = None;
        }
        if let Some(e) = &state.error {
            ui.add_space(8.0);
            ui.label(RichText::new(e).color(color(look.palette.status_danger_fg)));
        }
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = !state.command.trim().is_empty();
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if cr::primary_button(ui, look, tr("Save and run"), ready).clicked() || (enter && ready)
            {
                save = true;
            }
            if cr::secondary_button(ui, look, tr("Cancel"), true).clicked() {
                closed = true;
            }
        });
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if save {
        let Some(root) = work_root(app) else {
            state.error = Some(tr("Open a folder first.").to_owned());
            return (Overlay::RunSetup(state), false);
        };
        match tasks::save(&root, state.kind, &state.command) {
            Ok(()) => {
                let kind = state.kind;
                let ctx = ctx.clone();
                crate::terminal_ui::run_task(app, &ctx, kind);
                return (Overlay::None, true);
            }
            Err(e) => state.error = Some(e),
        }
    }
    (Overlay::RunSetup(state), false)
}

pub fn debug_setup(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut state: DebugSetup,
) -> Shown {
    let mut save = false;
    let mut closed = false;
    let dismissed = modal(ctx, look, "debug-setup", 540.0, false, |ui| {
        closed = heading(
            ui,
            look,
            tr("What should the debugger start?"),
            "debug-setup",
        );
        ui.label(
            RichText::new(tr(
                "Say which program to debug. Código remembers it in the project's .codigo folder. A native executable, a Python script or a Go file work.",
            ))
            .color(color(look.palette.text_secondary)),
        );
        if !state.note.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(&state.note).color(color(look.palette.status_warning_fg)));
        }
        ui.add_space(12.0);
        caption(ui, look, tr("Program"));
        let response = field(ui, look, &mut state.program, tr("build/app, main.py…"));
        if state.focus {
            response.request_focus();
            state.focus = false;
        }
        ui.add_space(10.0);
        caption(ui, look, tr("Arguments"));
        field(ui, look, &mut state.args, tr("optional"));
        ui.add_space(10.0);
        caption(ui, look, tr("Build command"));
        field(
            ui,
            look,
            &mut state.build,
            tr("optional, run before debugging"),
        );
        if response.changed() {
            state.error = None;
        }
        if let Some(e) = &state.error {
            ui.add_space(8.0);
            ui.label(RichText::new(e).color(color(look.palette.status_danger_fg)));
        }
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = !state.program.trim().is_empty();
            if cr::primary_button(ui, look, tr("Save and debug"), ready).clicked() {
                save = true;
            }
            if cr::secondary_button(ui, look, tr("Cancel"), true).clicked() {
                closed = true;
            }
        });
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if save {
        let Some(root) = work_root(app) else {
            state.error = Some(tr("Open a folder first.").to_owned());
            return (Overlay::DebugSetup(state), false);
        };
        let program = state.program.trim().to_owned();
        let build = state.build.trim();
        let saved = Saved {
            language: codigo_dap::language_of(&program).to_owned(),
            program,
            args: codigo_dap::split_args(&state.args),
            build: (!build.is_empty()).then(|| build.to_owned()),
        };
        match codigo_dap::save(&root, &saved) {
            Ok(()) => {
                crate::debug_ui::start(app);
                return (Overlay::None, true);
            }
            Err(e) => state.error = Some(e),
        }
    }
    (Overlay::DebugSetup(state), false)
}
