// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `PATH` a person's own shell would give.
//!
//! A program started from the Finder, the Dock or a desktop launcher does not get the `PATH` that
//! a terminal gets: `~/.cargo/bin`, Homebrew's folders and version managers are added by the
//! shell's start-up files, so `cargo`, `rustc` or a language server are "not found" from inside
//! the editor while working in a terminal. Código asks the login shell for its `PATH` once, at
//! start-up, and adopts it.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const START: &str = "__CODIGO_PATH_START__";
const END: &str = "__CODIGO_PATH_END__";

/// How long the shell may take to start; a slow start-up file must not delay the window.
const TIMEOUT: Duration = Duration::from_millis(2500);

/// The `PATH` printed between the markers, ignoring whatever else start-up files print.
pub fn parse(output: &str) -> Option<String> {
    let after = output.split_once(START)?.1;
    let path = after.split_once(END)?.0.trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// The login `PATH` first, then what the program already had that it does not list.
pub fn merge(login: &str, current: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in login.split(':').chain(current.split(':')) {
        if !part.is_empty() && !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts.join(":")
}

/// The options that make `shell` run a command as a login, interactive shell, where it has them.
fn flags(shell: &str) -> &'static str {
    let name = shell.rsplit('/').next().unwrap_or(shell);
    match name {
        "bash" | "zsh" | "fish" | "ksh" => "-ilc",
        _ => "-c",
    }
}

/// The command that prints `PATH` between the markers.
fn script() -> String {
    format!("printf '%s' '{START}'; printf '%s' \"$PATH\"; printf '%s' '{END}'")
}

/// Run `script` in `shell` with `flags`, giving up after `timeout`.
fn ask(shell: &str, flags: &str, script: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new(shell)
        .args([flags, script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if began.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse(&out)
}

/// What `shell` says its `PATH` is.
pub fn login_path(shell: &str) -> Option<String> {
    ask(shell, flags(shell), &script(), TIMEOUT)
}

/// Make this program's `PATH` the one a terminal would have, when it was not started from one.
/// Call it first thing, before any thread exists.
pub fn adopt() {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            return;
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| cfg!(target_os = "macos").then(|| "/bin/zsh".to_owned()));
        let Some(login) = shell.as_deref().and_then(login_path) else {
            return;
        };
        let current = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", merge(&login, &current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_read_between_the_markers_and_noise_is_ignored() {
        let out = format!("Welcome!\nmotd\n{START}/a:/b{END}\ngoodbye");
        assert_eq!(parse(&out).as_deref(), Some("/a:/b"));
        assert_eq!(parse("nothing"), None);
        assert_eq!(parse(&format!("{START}{END}")), None);
        assert_eq!(parse(&format!("{START}/a")), None);
    }

    #[test]
    fn the_login_path_comes_first_without_repeats() {
        assert_eq!(
            merge("/h/.cargo/bin:/usr/bin", "/usr/bin:/bin"),
            "/h/.cargo/bin:/usr/bin:/bin"
        );
        assert_eq!(merge("", "/bin"), "/bin");
        assert_eq!(merge("/a::/b", ""), "/a:/b");
    }

    #[test]
    fn only_shells_that_have_the_options_get_them() {
        assert_eq!(flags("/bin/zsh"), "-ilc");
        assert_eq!(flags("/usr/local/bin/fish"), "-ilc");
        assert_eq!(flags("/bin/sh"), "-c");
        assert_eq!(flags("/bin/dash"), "-c");
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_is_asked_for_its_path() {
        let path = ask("/bin/sh", "-c", &script(), Duration::from_secs(5)).expect("a path");
        assert!(!path.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_that_hangs_is_given_up_on() {
        let began = Instant::now();
        assert_eq!(
            ask("/bin/sh", "-c", "sleep 30", Duration::from_millis(200)),
            None
        );
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_shell_is_not_an_error() {
        assert_eq!(login_path("/no/such/shell"), None);
    }
}
