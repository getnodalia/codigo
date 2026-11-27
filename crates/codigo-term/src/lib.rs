// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's terminal. A [`Terminal`] runs a shell, or any command, in a pseudo-terminal and keeps
//! the screen it draws, so the same code serves the integrated terminal and the run console. No
//! UI here: the app reads the screen and feeds keys through [`keys`].

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod keys;
pub mod links;
mod terminal;

pub use terminal::{shell_command, shell_command_with, Spec, Status, TermError, Terminal, Waker};
pub use vt100::{Cell, Color, Screen};
