// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's syntax layer: which language a file is, and a tree-sitter tree that follows the text
//! as it is edited and says which parts of a line are keywords, strings, comments and so on.
//!
//! The grammars are tree-sitter's (C parsers built by the `cc` crate, MIT); everything around
//! them is Rust. A language without a grammar still has a [`LanguageDef`]: comment markers,
//! brackets and the name a language server knows it by.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod highlight;
pub mod language;
pub mod scope;

pub use highlight::{Span, Syntax};
pub use language::{detect, detect_with_first_line, find, languages, plain, LanguageDef};
pub use scope::Scope;
