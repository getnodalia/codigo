// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the token lives: the system's keyring (Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows), one entry per GitLab host, and nowhere else. When there is no
//! keyring, the caller keeps the token for the session only; it is never written to a file.

use crate::{Error, Result};

const SERVICE: &str = "dev.danielmiguelt.Codigo";

fn entry(host: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("gitlab:{host}"))
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// Keep the token for `host` in the keyring.
pub fn store(host: &str, token: &str) -> Result<()> {
    entry(host)?
        .set_password(token)
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// The saved token for `host`, if there is one and a keyring to ask.
pub fn load(host: &str) -> Option<String> {
    entry(host)
        .ok()?
        .get_password()
        .ok()
        .filter(|t| !t.is_empty())
}

/// Forget the saved token for `host`.
pub fn delete(host: &str) -> Result<()> {
    match entry(host)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keyring(e.to_string())),
    }
}
