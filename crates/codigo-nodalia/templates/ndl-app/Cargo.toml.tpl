[package]
name = "{{slug}}"
description = "{{name}}, an app in the Nodalia Design Language."
version = "0.1.0-dev"
edition = "2021"
rust-version = "1.91"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
eframe = { version = "0.37", default-features = false, features = ["accesskit", "clipboard", "default_fonts", "glow", "wayland", "x11"] }
egui = "0.37"
nodalia-theme = { git = "https://github.com/getnodalia/nodalia-design", rev = "{{rev}}", features = ["egui", "creator"] }
sys-locale = "0.3"

[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
unreachable = "deny"
