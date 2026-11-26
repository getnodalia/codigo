// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's text engine.
//!
//! A [`Buffer`] is a rope with several cursors, an undo history and the line-oriented editing
//! operations an editor needs. It does no I/O and knows nothing about drawing: positions are
//! character offsets into the rope, and every edit reports the [`Change`]s it made so the syntax
//! tree and a language server can follow along.
//!
//! Lines end with `\n` inside the buffer; a file's own line ending is detected when it is read
//! ([`encoding::decode`]) and written back when it is saved ([`encoding::encode`]).

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod buffer;
pub mod edit;
pub mod encoding;
pub mod history;
pub mod motion;
pub mod search;
pub mod selection;

pub use buffer::{Buffer, Indent, IndentRules, Motion, Pairs};
pub use edit::{Change, Edit};
pub use encoding::{decode, encode, DecodeError, Decoded, LineEnding};
pub use search::{find_all, Match, Query};
pub use selection::{Selection, Selections};
