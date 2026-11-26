// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's GitHub client, as a library with no interface.
//!
//! Everything here blocks, so call it off the UI thread. The token is the only secret: it is
//! kept in the system's keyring ([`secret`]), is sent only to `api.github.com` and
//! `github.com`, and never appears in an error or a log line.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod api;
pub mod device;
mod http;
mod model;
pub mod secret;

pub use api::{Api, ReviewEvent, Scope};
pub use model::{
    parse_remote, Check, Comment, CommentKind, Issue, Pull, PullDetail, PullFile, RepoInfo, User,
};

/// What can go wrong talking to GitHub.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub did not accept the token; sign in again")]
    Unauthorized,
    #[error("GitHub's request limit is spent; try again in a few minutes")]
    RateLimited,
    #[error("not found, or the token cannot see it")]
    NotFound,
    #[error("GitHub says: {0}")]
    Api(String),
    #[error("GitHub sent something unexpected: {0}")]
    Unexpected(String),
    #[error("{0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, Error>;
