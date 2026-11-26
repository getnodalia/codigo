// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's engine: everything an editor does that is not drawing it.
//!
//! A frontend asks for [`Document`]s, runs [`Cmd`]s, reads and writes [`Settings`] and keeps a
//! [`SessionState`]; it never touches the filesystem formats itself. That is what lets a terminal
//! frontend, a headless tool or an agent use the same code as the window.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod command;
pub mod document;
pub mod lsp;
pub mod paths;
pub mod session;
pub mod settings;

pub use codigo_lsp::Severity;
pub use command::{Category, Cmd, Key, Shortcut};
pub use document::{detect_indent, DocId, Document, OpenError, SaveError};
pub use lsp::{Counts, Hub, Mark, ServerState, ServerStatus};
pub use session::SessionState;
pub use settings::{Appearance, AutoSave, Settings};
