// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window: colours come from the Nodalia tokens, text goes through `i18n`.

use egui::{Id, RichText};
use nodalia_theme::about::{self, Info, Link};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::for_egui::{self, color, Appearance};
use nodalia_theme::{font_size, Accent, Mode};

use crate::i18n::{tr, trf};

pub const NAME: &str = "{{name}}";
const ACCENT: Accent = Accent::{{accent}};

pub struct App {
    clicks: u32,
    about: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        for_egui::apply_system(
            &cc.egui_ctx,
            &Appearance {
                accent: ACCENT,
                ..Appearance::default()
            },
        );
        App {
            clicks: 0,
            about: false,
        }
    }
}

/// The colours to draw with, following the system's light or dark theme.
fn look(ctx: &egui::Context) -> Look {
    let mode = if ctx.theme() == egui::Theme::Dark {
        Mode::Dark
    } else {
        Mode::Light
    };
    Look::new(mode.palette_with(ACCENT))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let look = look(&ctx);
        let p = look.palette;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(color(p.surface_window))
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                cr::island(&look).inner_margin(24).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(tr("Welcome"))
                            .strong()
                            .size(font_size::TITLE)
                            .color(color(p.text_primary)),
                    );
                    ui.label(
                        RichText::new(tr("This app is drawn in the Nodalia Design Language."))
                            .color(color(p.text_secondary)),
                    );
                    ui.add_space(16.0);
                    if cr::primary_button(ui, &look, tr("Click me"), true).clicked() {
                        self.clicks += 1;
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(trf("Clicked {} times", &[&self.clicks.to_string()]))
                            .color(color(p.text_tertiary)),
                    );
                    ui.add_space(16.0);
                    if cr::secondary_button(ui, &look, tr("About"), true).clicked() {
                        self.about = true;
                    }
                });
            });
        if self.about {
            self.about = !about_window(&ctx, &look);
        }
    }
}

/// The shared About window; true once it should close.
fn about_window(ctx: &egui::Context, look: &Look) -> bool {
    let version = trf("Version {}", &[env!("CARGO_PKG_VERSION")]);
    let links = [Link {
        label: tr("Website"),
        url: "https://getnodalia.com",
    }];
    let info = Info {
        name: NAME,
        version: &version,
        tagline: tr("An app built with Código."),
        suite: tr("Part of Nodalia"),
        links: &links,
        footer: "MIT OR Apache-2.0",
    };
    let mut closed = false;
    let frame = cr::island(look).inner_margin(16);
    let response = egui::Modal::new(Id::unique("about"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(about::measures::WIDTH);
            closed = about::heading(ui, look, tr("About"), Id::unique("about-close")).closed;
            let page = about::page(ui, look, &info, |ui, rect| {
                ui.painter()
                    .rect_filled(rect, 20.0, color(look.palette.accent_default));
            });
            if let Some(link) = page.picked.and_then(|i| links.get(i)) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(link.url));
            }
        });
    closed || response.should_close()
}
