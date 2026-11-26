// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's language server client.
//!
//! Nothing here draws anything or touches the UI thread: a [`Client`] owns a reader thread and
//! hands what the server says to the editor through a channel, and every request returns a handle
//! that is polled, never waited on. A server is a stranger's program: its messages are bounded,
//! malformed ones are skipped, and a server that dies is reported as an event, not a crash.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod client;
pub mod diagnostics;
pub mod framing;
pub mod position;
pub mod requests;
pub mod servers;

pub use client::{Client, ClientError, Event, Pending, ResponseError, Waker};
pub use diagnostics::{Diagnostic, Severity};
pub use position::{char_to_utf16, path_to_uri, uri_to_path, utf16_to_char, Position, Range};
pub use requests::{
    parse_completion, parse_hover, parse_locations, parse_text_edits, parse_workspace_edit,
    strip_snippet, CompletionItem, FileEdits, Location, TextEdit,
};
pub use servers::{
    find_executable, server_for, spawn, spawn_with, ServerSpec, SpawnError, Spawned, SERVERS,
};
