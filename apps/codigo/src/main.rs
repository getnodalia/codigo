// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's desktop app: native egui, drawn in the Nodalia Design Language.
//!
//! `CODIGO_CONFIG_DIR` keeps settings and the session somewhere else than the platform's
//! config folder, which is how two copies run side by side on one computer.

// A window app on Windows: no console opens beside it.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod app;
mod chrome;
mod editor;
mod explorer;
mod file_icons;
mod i18n;
mod icon;
mod layout;
#[cfg(target_os = "linux")]
mod linux_menu;
mod lsp_ui;
mod menu;
mod overlays;
mod spanish;
mod syntax_theme;
mod ui;
mod updates;
mod widgets;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut open: Vec<std::path::PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("codigo {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: codigo [FOLDER|FILE...]\n\nCODIGO_CONFIG_DIR=<folder> keeps settings and the session in <folder>."
                );
                return ExitCode::SUCCESS;
            }
            path => open.push(path.into()),
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let settings = codigo_engine::Settings::load();
    let ndl_controls = chrome::NDL_CONTROLS_POSSIBLE && !settings.native_window_controls;
    let mut viewport = chrome::frame(
        egui::ViewportBuilder::default()
            .with_title(app::NAME)
            .with_app_id("dev.danielmiguelt.Codigo")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([760.0, 480.0]),
        ndl_controls,
    );
    if let Some(icon) = icon::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        app::NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, &open)) as Box<dyn eframe::App>)),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("codigo: {e}");
            ExitCode::FAILURE
        }
    }
}
