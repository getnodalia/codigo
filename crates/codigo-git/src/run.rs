// SPDX-License-Identifier: MIT OR Apache-2.0
//! Running git.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::{base64, Error, Result};

/// How a command that talks to a remote proves who the person is.
#[derive(Clone, Default)]
pub enum Auth {
    /// Whatever git is configured to do (SSH keys, a credential helper).
    #[default]
    None,
    /// A GitHub token, sent for https remotes of github.com only.
    GitHubToken(String),
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::None => f.write_str("Auth::None"),
            Auth::GitHubToken(_) => f.write_str("Auth::GitHubToken(…)"),
        }
    }
}

/// The most output kept from a command.
const MAX_OUTPUT: usize = 16 * 1024 * 1024;

pub fn git(dir: &Path, args: &[&str], auth: &Auth) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=off"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    if let Auth::GitHubToken(token) = auth {
        // Through the environment, so the token is in no argument list and no file.
        let basic = base64::encode(format!("x-access-token:{token}").as_bytes());
        cmd.env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Basic {basic}"),
            );
    }
    let out = cmd
        .output()
        .map_err(|e| Error::NotInstalled(e.to_string()))?;
    if !out.status.success() {
        let mut text = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if text.is_empty() {
            text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        }
        if text.is_empty() {
            text = format!("git {} failed", args.first().copied().unwrap_or(""));
        }
        return Err(Error::Failed(scrub(&text, auth)));
    }
    let mut stdout = out.stdout;
    stdout.truncate(MAX_OUTPUT);
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Error text never carries the token, whatever git decides to print.
pub(crate) fn scrub(text: &str, auth: &Auth) -> String {
    match auth {
        Auth::GitHubToken(t) if t.len() >= 6 => text.replace(t.as_str(), "***"),
        _ => text.to_owned(),
    }
}
