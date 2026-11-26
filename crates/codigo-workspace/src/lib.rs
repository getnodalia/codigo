// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's view of a project folder: its file tree, a fuzzy matcher for quick open, search across
//! files, and what git says about them.
//!
//! Nothing here draws anything, and nothing here is allowed to hang the editor: every walk takes a
//! cancel flag and a limit, and git is only ever asked through the command line.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod fuzzy;
pub mod git;
pub mod search;
pub mod tree;
pub mod walk;

pub use fuzzy::{rank, Ranked, Scorer};
pub use git::{line_changes, status as git_status, GitInfo, LineChange, LineChangeKind, Status};
pub use search::{search, Hit, SearchOptions, SearchResult};
pub use tree::{read_dir, Entry};
pub use walk::list_files;
