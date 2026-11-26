// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Nodalia menu: tools for building Nodalia apps. For now, the "New Nodalia app" window.

use std::path::PathBuf;

use codigo_nodalia::{Accent, AppSpec, Error};
use egui::{Align, Align2, Layout, RichText};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};

/// The state of the "New Nodalia app" window.
pub struct NewApp {
    name: String,
    accent: Accent,
    /// The folder the app's own folder is created in.
    parent: PathBuf,
    error: Option<String>,
    focus: bool,
}

impl NewApp {
    pub fn new(app: &App) -> NewApp {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let parent = app
            .project
            .as_ref()
            .and_then(|p| p.root.parent().map(PathBuf::from))
            .or(home)
            .unwrap_or_else(|| PathBuf::from("."));
        NewApp {
            name: String::new(),
            accent: Accent::Nodalia,
            parent,
            error: None,
            focus: true,
        }
    }
}

fn error_text(e: &Error) -> String {
    match e {
        Error::BadName => tr("Use letters, digits and spaces, starting with a letter.").to_owned(),
        Error::Exists(p) => trf("{} already exists.", &[&p.display().to_string()]),
        Error::Io(e) => trf("Could not create the app: {}", &[e]),
    }
}

pub fn new_app(ctx: &egui::Context, app: &mut App, look: &Look, mut state: NewApp) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut create = false;
    let mut choose = false;
    let dismissed = modal(ctx, look, "new-app", 540.0, false, |ui| {
        closed = heading(ui, look, tr("New Nodalia app"), "new-app");
        ui.label(
            RichText::new(tr(
                "Creates an egui app in the Nodalia Design Language with the shared theme, an About window, Spanish and English, the lints and the CI of the other apps.",
            ))
            .color(color(p.text_secondary)),
        );
        ui.add_space(12.0);
        ui.label(
            RichText::new(tr("Name"))
                .strong()
                .size(font_size::BODY)
                .color(color(p.text_secondary)),
        );
        let field = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.name)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("My app"))
                        .align(Align2::LEFT_CENTER)
                        .char_limit(codigo_nodalia::MAX_NAME)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(font_size::LARGE)),
                )
            })
            .inner;
        if state.focus {
            field.request_focus();
            state.focus = false;
        }
        if field.changed() {
            state.error = None;
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(tr("Colour"))
                .strong()
                .size(font_size::BODY)
                .color(color(p.text_secondary)),
        );
        egui::ComboBox::from_id_salt("new-app-accent")
            .selected_text(format!("{:?}", state.accent))
            .show_ui(ui, |ui| {
                for a in Accent::ALL {
                    ui.selectable_value(&mut state.accent, a, format!("{a:?}"));
                }
            });
        ui.add_space(10.0);
        ui.label(
            RichText::new(tr("Folder"))
                .strong()
                .size(font_size::BODY)
                .color(color(p.text_secondary)),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(state.parent.display().to_string()).monospace());
            if cr::secondary_button(ui, look, tr("Choose…"), true).clicked() {
                choose = true;
            }
        });
        if let Some(e) = &state.error {
            ui.add_space(8.0);
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = codigo_nodalia::valid_name(&state.name);
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if cr::primary_button(ui, look, tr("Create"), ready).clicked() || (enter && ready) {
                create = true;
            }
        });
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if choose {
        if let Some(dir) = rfd::FileDialog::new()
            .set_directory(&state.parent)
            .pick_folder()
        {
            state.parent = dir;
        }
    }
    if create {
        let spec = AppSpec {
            name: state.name.clone(),
            accent: state.accent,
        };
        match codigo_nodalia::create_app(&state.parent, &spec) {
            Ok(made) => {
                app.open_project(made.root.clone(), true);
                app.open_path(&made.root.join(&made.entry), None, true);
                app.toast(trf("Created {}", &[spec.name.trim()]));
                return (Overlay::None, true);
            }
            Err(e) => state.error = Some(error_text(&e)),
        }
    }
    (Overlay::NewApp(state), false)
}
