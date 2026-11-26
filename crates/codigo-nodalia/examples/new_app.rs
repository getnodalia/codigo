// SPDX-License-Identifier: MIT OR Apache-2.0
//! `cargo run -p codigo-nodalia --example new_app -- <parent folder> <name>` creates an app.

use codigo_nodalia::{create_app, Accent, AppSpec};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(parent), Some(name)) = (args.next(), args.next()) else {
        eprintln!("usage: new_app <parent folder> <name>");
        return;
    };
    match create_app(
        std::path::Path::new(&parent),
        &AppSpec {
            name,
            accent: Accent::Nodalia,
        },
    ) {
        Ok(made) => println!("{}", made.root.display()),
        Err(e) => eprintln!("{e}"),
    }
}
