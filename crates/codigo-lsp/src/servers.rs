// SPDX-License-Identifier: MIT OR Apache-2.0
//! The language servers Código knows how to start, and finding and launching them.
//!
//! Código never downloads or installs a server: it looks for the program on the user's `PATH`
//! (and in `~/.cargo/bin`, where `rustup component add rust-analyzer` puts it) and says what to
//! install when it is missing.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::client::Client;

#[derive(Debug)]
pub struct ServerSpec {
    /// The language ids (as in `codigo-syntax`) this server handles.
    pub languages: &'static [&'static str],
    pub command: &'static str,
    pub args: &'static [&'static str],
    /// A line telling the user how to get it, shown when the program is not found.
    pub install: &'static str,
}

pub const SERVERS: &[ServerSpec] = &[
    ServerSpec {
        languages: &["rust"],
        command: "rust-analyzer",
        args: &[],
        install: "rustup component add rust-analyzer",
    },
    ServerSpec {
        languages: &["python"],
        command: "pyright-langserver",
        args: &["--stdio"],
        install: "npm install -g pyright",
    },
    ServerSpec {
        languages: &["javascript", "typescript"],
        command: "typescript-language-server",
        args: &["--stdio"],
        install: "npm install -g typescript typescript-language-server",
    },
    ServerSpec {
        languages: &["go"],
        command: "gopls",
        args: &[],
        install: "go install golang.org/x/tools/gopls@latest",
    },
    ServerSpec {
        languages: &["c", "cpp"],
        command: "clangd",
        args: &[],
        install: "install clangd with your package manager",
    },
    ServerSpec {
        languages: &["java"],
        command: "jdtls",
        args: &[],
        install: "install Eclipse JDT LS (jdtls)",
    },
    ServerSpec {
        languages: &["json"],
        command: "vscode-json-language-server",
        args: &["--stdio"],
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        languages: &["html"],
        command: "vscode-html-language-server",
        args: &["--stdio"],
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        languages: &["css"],
        command: "vscode-css-language-server",
        args: &["--stdio"],
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerSpec {
        languages: &["yaml"],
        command: "yaml-language-server",
        args: &["--stdio"],
        install: "npm install -g yaml-language-server",
    },
    ServerSpec {
        languages: &["bash"],
        command: "bash-language-server",
        args: &["start"],
        install: "npm install -g bash-language-server",
    },
    ServerSpec {
        languages: &["toml"],
        command: "taplo",
        args: &["lsp", "stdio"],
        install: "cargo install taplo-cli --locked",
    },
];

/// The server for a language id, if Código knows one.
pub fn server_for(language: &str) -> Option<&'static ServerSpec> {
    SERVERS.iter().find(|s| s.languages.contains(&language))
}

/// Look for `name` on `PATH` and in `~/.cargo/bin`. A name with a path separator is refused:
/// the registry only ever holds bare program names.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(Path::new(&home).join(".cargo").join("bin"));
    }
    find_in(name, &dirs)
}

fn find_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    for dir in dirs {
        for ext in candidates {
            let path = dir.join(format!("{name}{ext}"));
            if is_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// A running server: the process and the client talking to it. Dropping it kills the process.
pub struct Spawned {
    pub client: Client,
    child: Child,
}

impl Spawned {
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Whether the process has already exited.
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("{command} is not installed ({install})")]
    NotFound {
        command: String,
        install: &'static str,
    },
    #[error("could not start {command}: {source}")]
    Start {
        command: String,
        source: std::io::Error,
    },
}

/// Start `spec` with `cwd` as its working directory and connect a client to it.
pub fn spawn(spec: &ServerSpec, cwd: &Path) -> Result<Spawned, SpawnError> {
    let program = find_executable(spec.command).ok_or_else(|| SpawnError::NotFound {
        command: spec.command.to_owned(),
        install: spec.install,
    })?;
    spawn_program(&program, spec.args, cwd, spec.command)
}

fn spawn_program(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    name: &str,
) -> Result<Spawned, SpawnError> {
    let start_err = |source| SpawnError::Start {
        command: name.to_owned(),
        source,
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(start_err)?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Err(SpawnError::Start {
            command: name.to_owned(),
            source: std::io::Error::other("no pipes"),
        });
    };
    if let Some(stderr) = child.stderr.take() {
        let label = name.to_owned();
        let _ = std::thread::Builder::new()
            .name("codigo-lsp-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr)
                    .lines()
                    .map_while(Result::ok)
                    .take(10_000)
                {
                    tracing::debug!("{label}: {line}");
                }
            });
    }
    Ok(Spawned {
        client: Client::new(stdout, stdin),
        child,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_at_most_one_server_and_names_are_bare() {
        for s in SERVERS {
            assert!(!s.command.contains('/') && !s.command.is_empty());
            for l in s.languages {
                assert_eq!(
                    SERVERS.iter().filter(|o| o.languages.contains(l)).count(),
                    1,
                    "{l}"
                );
            }
        }
        assert_eq!(server_for("rust").map(|s| s.command), Some("rust-analyzer"));
        assert!(server_for("brainfuck").is_none());
    }

    #[test]
    fn path_separators_in_a_name_are_refused() {
        assert!(find_executable("../sh").is_none());
        assert!(find_executable("/bin/sh").is_none());
        assert!(find_executable("").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn finds_only_executable_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().expect("tmp");
        let plain = d.path().join("tool");
        std::fs::write(&plain, "x").expect("w");
        assert!(find_in("tool", &[d.path().to_path_buf()]).is_none());
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert_eq!(find_in("tool", &[d.path().to_path_buf()]), Some(plain));
    }

    #[test]
    #[cfg(unix)]
    fn a_program_that_exits_is_reported_as_exited() {
        let d = tempfile::tempdir().expect("tmp");
        let mut s =
            spawn_program(Path::new("/bin/sh"), &["-c", "exit 0"], d.path(), "sh").expect("spawn");
        let mut tries = 0;
        while !s.exited() && tries < 200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            tries += 1;
        }
        assert!(s.exited());
        let saw = s
            .client
            .events()
            .recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(saw, Ok(crate::client::Event::Exited));
    }

    #[test]
    fn a_missing_server_says_how_to_get_it() {
        let spec = ServerSpec {
            languages: &["x"],
            command: "definitely-not-installed-codigo",
            args: &[],
            install: "install it",
        };
        let err = spawn(&spec, Path::new("/")).err().map(|e| e.to_string());
        assert_eq!(
            err.as_deref(),
            Some("definitely-not-installed-codigo is not installed (install it)")
        );
    }
}
