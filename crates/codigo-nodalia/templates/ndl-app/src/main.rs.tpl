// SPDX-License-Identifier: MIT OR Apache-2.0
//! {{name}}: a native app drawn in the Nodalia Design Language.

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
mod i18n;
mod spanish;

use std::process::ExitCode;

fn main() -> ExitCode {
    i18n::set(i18n::Lang::system());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(app::NAME)
            .with_app_id("{{app_id}}")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        app::NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)) as Box<dyn eframe::App>)),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{{slug}}: {e}");
            ExitCode::FAILURE
        }
    }
}
