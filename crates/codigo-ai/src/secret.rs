// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where an API key lives: the system's keyring (Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows), and nowhere else. When there is no keyring, the caller keeps
//! the key for the session only; it is never written to a file.

use crate::{Error, Provider, Result};

const SERVICE: &str = "dev.danielmiguelt.Codigo";

fn entry(provider: Provider) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("ai-{}", provider.id()))
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// Keep the key for a provider in the keyring.
pub fn store(provider: Provider, key: &str) -> Result<()> {
    entry(provider)?
        .set_password(key)
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// The saved key, if there is one and a keyring to ask.
pub fn load(provider: Provider) -> Option<String> {
    entry(provider)
        .ok()?
        .get_password()
        .ok()
        .filter(|k| !k.is_empty())
}

/// Forget the saved key.
pub fn delete(provider: Provider) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keyring(e.to_string())),
    }
}

/// A key as typed or pasted: no spaces or line breaks, which would make a header invalid.
pub fn clean(typed: &str) -> Option<String> {
    let k = typed.trim();
    (!k.is_empty() && !k.contains(char::is_whitespace) && k.len() <= 512).then(|| k.to_owned())
}
