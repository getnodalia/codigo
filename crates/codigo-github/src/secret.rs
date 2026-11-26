// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the token lives: the system's keyring (Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows), and nowhere else. When there is no keyring, the caller keeps
//! the token for the session only; it is never written to a file.

use std::process::{Command, Stdio};

use crate::{Error, Result};

const SERVICE: &str = "dev.danielmiguelt.Codigo";
const ACCOUNT: &str = "github";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| Error::Keyring(e.to_string()))
}

/// Keep the token in the keyring.
pub fn store(token: &str) -> Result<()> {
    entry()?
        .set_password(token)
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// The saved token, if there is one and a keyring to ask.
pub fn load() -> Option<String> {
    entry().ok()?.get_password().ok().filter(|t| !t.is_empty())
}

/// Forget the saved token.
pub fn delete() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keyring(e.to_string())),
    }
}

/// The token of the GitHub CLI (`gh`), when the person is signed in there. Asked for only when
/// the person chooses it.
pub fn from_gh_cli() -> Option<String> {
    let out = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!token.is_empty() && !token.contains(char::is_whitespace)).then_some(token)
}
