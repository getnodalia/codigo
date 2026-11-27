// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's GitLab client, as a library with no interface. It talks to gitlab.com or to a
//! GitLab server the person names, with a personal access token.
//!
//! Everything here blocks, so call it off the UI thread. The token is the only secret: it is
//! kept in the system's keyring ([`secret`]), is sent only to the one host it was made for, over
//! HTTPS and without following redirects, and never appears in an error or a log line.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod api;
mod http;
mod model;
pub mod secret;

pub use api::{valid_host, Api, NewMergeRequest, Scope, DEFAULT_HOST};
pub use model::{parse_remote, Change, Issue, MergeRequest, Note, Project, User};

/// What can go wrong talking to GitLab.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("could not reach GitLab: {0}")]
    Network(String),
    #[error("GitLab did not accept the token; sign in again")]
    Unauthorized,
    #[error("GitLab's request limit is spent; try again in a few minutes")]
    RateLimited,
    #[error("not found, or the token cannot see it")]
    NotFound,
    #[error("GitLab says: {0}")]
    Api(String),
    #[error("GitLab sent something unexpected: {0}")]
    Unexpected(String),
    #[error("{0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, Error>;
