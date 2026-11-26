// SPDX-License-Identifier: MIT OR Apache-2.0
//! The debug adapters Código knows how to start.
//!
//! Like the language servers, an adapter is a program the person installs: Código looks for it on
//! `PATH` and says what to install when it is missing.

use std::path::{Path, PathBuf};

use codigo_lsp::find_executable;
use serde_json::{json, Value};

/// How an adapter wants its `launch` request written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    /// `lldb-dap`, for Rust, C and C++.
    Lldb,
    /// `gdb --interpreter=dap` (GDB 14 and newer), for Rust, C and C++.
    Gdb,
    /// `python -m debugpy.adapter`.
    Debugpy,
    /// `dlv dap`, for Go.
    Delve,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub kind: AdapterKind,
    /// What the person sees, e.g. `lldb-dap`.
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// No adapter for a language is installed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("no debugger found for {language}: {install}")]
pub struct Missing {
    pub language: String,
    /// A line telling the person how to get one.
    pub install: &'static str,
}

impl AdapterKind {
    /// GDB starts the program as soon as it has the `launch` request, so the breakpoints must
    /// follow it at once rather than after the `initialized` event, and it answers
    /// `configurationDone` with an error when the program is already past that point.
    pub fn pipelined(self) -> bool {
        self == AdapterKind::Gdb
    }
}

impl Adapter {
    /// The arguments of the `launch` request for running `program` in `cwd`.
    pub fn launch_arguments(&self, program: &Path, args: &[String], cwd: &Path) -> Value {
        let (program, cwd) = (
            program.to_string_lossy().into_owned(),
            cwd.to_string_lossy().into_owned(),
        );
        match self.kind {
            AdapterKind::Lldb => json!({
                "program": program, "args": args, "cwd": cwd, "stopOnEntry": false
            }),
            AdapterKind::Gdb => json!({
                "program": program, "args": args, "cwd": cwd,
                "stopAtBeginningOfMainSubprogram": true
            }),
            AdapterKind::Debugpy => json!({
                "type": "python", "request": "launch", "program": program, "args": args,
                "cwd": cwd, "console": "internalConsole", "justMyCode": false
            }),
            AdapterKind::Delve => json!({
                "mode": "debug", "program": program, "args": args, "cwd": cwd
            }),
        }
    }
}

/// `lldb-dap`, or the versioned name distributions use (`lldb-dap-18`), or the older
/// `lldb-vscode`.
fn find_lldb() -> Option<PathBuf> {
    if let Some(p) = find_executable("lldb-dap").or_else(|| find_executable("lldb-vscode")) {
        return Some(p);
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = name
                .strip_prefix("lldb-dap-")
                .or_else(|| name.strip_prefix("lldb-vscode-"))
                .and_then(|v| v.parse::<u32>().ok());
            if let Some(v) = version {
                found.push((v, entry.path()));
            }
        }
    }
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// True when `gdb` is new enough to speak the Debug Adapter Protocol (14 and newer).
fn gdb_has_dap(program: &Path) -> bool {
    let Ok(out) = std::process::Command::new(program)
        .arg("--version")
        .output()
    else {
        return false;
    };
    gdb_major(&String::from_utf8_lossy(&out.stdout)).is_some_and(|v| v >= 14)
}

/// The major version in the first line of `gdb --version`, which ends in the version number
/// (`GNU gdb (Ubuntu 15.1-1ubuntu1) 15.1`).
fn gdb_major(version_output: &str) -> Option<u32> {
    version_output
        .lines()
        .next()?
        .split_whitespace()
        .rev()
        .find_map(|word| word.split('.').next()?.parse::<u32>().ok())
}

/// Every adapter installed for a language id (as in `codigo-syntax`), the preferred one first.
pub fn adapters_for(language: &str) -> Result<Vec<Adapter>, Missing> {
    let mut found = Vec::new();
    match language {
        "rust" | "c" | "cpp" => {
            if let Some(program) = find_lldb() {
                found.push(Adapter {
                    kind: AdapterKind::Lldb,
                    name: "lldb-dap",
                    program,
                    args: Vec::new(),
                });
            }
            if let Some(program) = find_executable("gdb").filter(|p| gdb_has_dap(p)) {
                found.push(Adapter {
                    kind: AdapterKind::Gdb,
                    name: "gdb",
                    program,
                    args: vec!["--interpreter=dap".into()],
                });
            }
        }
        "python" => {
            if let Some(program) = find_executable("python3").or_else(|| find_executable("python"))
            {
                found.push(Adapter {
                    kind: AdapterKind::Debugpy,
                    name: "debugpy",
                    program,
                    args: vec!["-m".into(), "debugpy.adapter".into()],
                });
            }
        }
        "go" => {
            if let Some(program) = find_executable("dlv") {
                found.push(Adapter {
                    kind: AdapterKind::Delve,
                    name: "dlv",
                    program,
                    args: vec!["dap".into()],
                });
            }
        }
        _ => {}
    }
    if found.is_empty() {
        return Err(Missing {
            language: language.into(),
            install: match language {
                "rust" | "c" | "cpp" => {
                    "install lldb (lldb-dap) or GDB 14 or newer with your package manager"
                }
                "python" => "install Python, then run: python3 -m pip install debugpy",
                "go" => "go install github.com/go-delve/delve/cmd/dlv@latest",
                _ => "Código can debug Rust, C, C++, Python and Go so far",
            },
        });
    }
    Ok(found)
}

/// The preferred adapter for a language id.
pub fn adapter_for(language: &str) -> Result<Adapter, Missing> {
    adapters_for(language)?.into_iter().next().ok_or(Missing {
        language: language.into(),
        install: "Código can debug Rust, C, C++, Python and Go so far",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gdb_version_is_read_from_the_end_of_the_first_line() {
        assert_eq!(
            gdb_major("GNU gdb (Ubuntu 15.1-1ubuntu1~24.04.1) 15.1\nCopyright"),
            Some(15)
        );
        assert_eq!(gdb_major("GNU gdb (GDB) 13.2\n"), Some(13));
        assert_eq!(gdb_major(""), None);
        assert_eq!(gdb_major("nonsense"), None);
    }

    #[test]
    fn languages_without_a_debugger_say_so() {
        let e = adapter_for("brainfuck").expect_err("unsupported");
        assert!(e.to_string().contains("brainfuck"));
    }

    #[test]
    fn each_adapter_writes_its_own_launch_request() {
        let mk = |kind| Adapter {
            kind,
            name: "x",
            program: PathBuf::from("x"),
            args: Vec::new(),
        };
        let (p, c) = (Path::new("/p/app"), Path::new("/p"));
        let a = vec!["--flag".to_string()];
        let lldb = mk(AdapterKind::Lldb).launch_arguments(p, &a, c);
        assert_eq!(lldb["program"], "/p/app");
        assert_eq!(lldb["args"][0], "--flag");
        assert_eq!(lldb["stopOnEntry"], false);
        let py = mk(AdapterKind::Debugpy).launch_arguments(p, &a, c);
        assert_eq!(py["request"], "launch");
        let dlv = mk(AdapterKind::Delve).launch_arguments(p, &a, c);
        assert_eq!(dlv["mode"], "debug");
    }
}
