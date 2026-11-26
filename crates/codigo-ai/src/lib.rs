// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's AI assistants, as a library with no interface.
//!
//! The person picks the provider; Código ships no key and no account. A key lives in the system's
//! keyring ([`secret`]) and nowhere else, is sent only to the host of the provider it belongs to,
//! and never appears in an error. Nothing is sent until the person asks for something, and what
//! is sent is exactly what they chose to include: [`prompts`] builds it from the text it is given.
//! Everything here blocks, so call it off the UI thread.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod chat;
pub mod prompts;
mod provider;
pub mod secret;

pub use chat::{complete, Message, Role};
pub use provider::{Config, Provider};

/// What can go wrong asking an assistant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("could not reach the provider: {0}")]
    Network(String),
    #[error("the provider did not accept the key; check it in the AI settings")]
    Unauthorized,
    #[error("the provider's request limit is spent; try again in a moment")]
    RateLimited,
    #[error("the provider says: {0}")]
    Api(String),
    #[error("the provider sent something unexpected: {0}")]
    Unexpected(String),
    #[error("{0}")]
    Setup(String),
    #[error("{0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;
