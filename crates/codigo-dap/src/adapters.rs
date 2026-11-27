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

/// How a missing debugger can be had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// macOS without the command line tools: `xcode-select --install` opens Apple's installer.
    XcodeTools,
    /// macOS where the tools are there but too old to carry `lldb-dap` (Xcode 16 or newer has it).
    XcodeToolsOld,
    /// A command to run in a terminal (a package manager line).
    Command(String),
    /// Python itself is missing.
    Python,
    /// Nothing is known for this language.
    Unsupported,
}

impl Install {
    /// The line to run in a terminal, when there is one.
    pub fn command(&self) -> Option<String> {
        match self {
            Install::XcodeTools => Some("xcode-select --install".into()),
            Install::XcodeToolsOld => Some("brew install llvm".into()),
            Install::Command(c) => Some(c.clone()),
            Install::Python | Install::Unsupported => None,
        }
    }
}

/// No adapter for a language is installed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("no debugger found for {language}: {}", install_text(.install))]
pub struct Missing {
    pub language: String,
    /// How to get one.
    pub install: Install,
}

fn install_text(install: &Install) -> String {
    match install {
        Install::XcodeTools => {
            "install the Xcode command line tools (xcode-select --install)".into()
        }
        Install::XcodeToolsOld => {
            "update the Xcode command line tools to version 16 or newer, or run: brew install llvm"
                .into()
        }
        Install::Command(c) => format!("run: {c}"),
        Install::Python => "install Python, then run: python3 -m pip install debugpy".into(),
        Install::Unsupported => "Código can debug Rust, C, C++, Python and Go so far".into(),
    }
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

/// Folders besides `PATH` where a debugger is known to live: the Xcode tools and Homebrew on
/// macOS, LLVM's own folders on Linux (Debian and Ubuntu keep `lldb-dap` in `/usr/lib/llvm-N/bin`,
/// out of `PATH`), and `~/.cargo/bin`.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        let dev = std::process::Command::new("xcode-select")
            .arg("-p")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
        for root in dev.into_iter().chain([
            PathBuf::from("/Library/Developer/CommandLineTools"),
            PathBuf::from("/Applications/Xcode.app/Contents/Developer"),
        ]) {
            dirs.push(root.join("usr/bin"));
            dirs.push(root.join("Toolchains/XcodeDefault.xctoolchain/usr/bin"));
        }
        for brew in ["/opt/homebrew", "/usr/local"] {
            dirs.push(Path::new(brew).join("opt/llvm/bin"));
            dirs.push(Path::new(brew).join("bin"));
        }
    }
    for base in ["/usr/lib", "/usr/lib64", "/usr/local/lib"] {
        let Ok(read) = std::fs::read_dir(base) else {
            continue;
        };
        let mut found: Vec<PathBuf> = read
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("llvm"))
            .map(|e| e.path().join("bin"))
            .collect();
        found.sort();
        found.reverse();
        dirs.extend(found);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(Path::new(&home).join(".cargo").join("bin"));
    }
    dirs
}

/// `lldb-dap`, or the older `lldb-vscode`, or a versioned name (`lldb-dap-18`), in `dirs`; the
/// plain name wins over a versioned one and a newer version over an older one.
fn find_lldb_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        for name in ["lldb-dap", "lldb-vscode"] {
            let path = dir.join(name);
            if is_file(&path) {
                return Some(path);
            }
        }
    }
    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else {
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

fn is_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn find_lldb() -> Option<PathBuf> {
    if let Some(p) = find_executable("lldb-dap").or_else(|| find_executable("lldb-vscode")) {
        return Some(p);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(extra_dirs());
    find_lldb_in(&dirs)
}

/// `gdb`, on `PATH` or where Homebrew and friends put it.
fn find_gdb() -> Option<PathBuf> {
    find_executable("gdb").or_else(|| {
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .iter()
            .map(|d| Path::new(d).join("gdb"))
            .find(|p| is_file(p))
    })
}

/// What to tell someone who has no C-family debugger, from what this computer has.
fn native_install() -> Install {
    if cfg!(target_os = "macos") {
        let tools = std::process::Command::new("xcode-select")
            .arg("-p")
            .output()
            .is_ok_and(|o| o.status.success());
        return if tools {
            Install::XcodeToolsOld
        } else {
            Install::XcodeTools
        };
    }
    if cfg!(windows) {
        return Install::Command("winget install LLVM.LLVM".into());
    }
    let have = |name: &str| find_executable(name).is_some();
    let command = if have("apt-get") || have("apt") {
        "sudo apt install lldb"
    } else if have("dnf") {
        "sudo dnf install lldb"
    } else if have("pacman") {
        "sudo pacman -S lldb"
    } else if have("zypper") {
        "sudo zypper install lldb"
    } else if have("apk") {
        "sudo apk add lldb"
    } else {
        "install lldb (it includes lldb-dap) with your package manager"
    };
    Install::Command(command.into())
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
            if let Some(program) = find_gdb().filter(|p| gdb_has_dap(p)) {
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
                "rust" | "c" | "cpp" => native_install(),
                "python" => Install::Python,
                "go" => {
                    Install::Command("go install github.com/go-delve/delve/cmd/dlv@latest".into())
                }
                _ => Install::Unsupported,
            },
        });
    }
    Ok(found)
}

/// The preferred adapter for a language id.
pub fn adapter_for(language: &str) -> Result<Adapter, Missing> {
    adapters_for(language)?.into_iter().next().ok_or(Missing {
        language: language.into(),
        install: Install::Unsupported,
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
    fn lldb_is_found_by_any_of_its_names_and_the_newest_version_wins() {
        let a = tempfile::tempdir().expect("dir");
        let b = tempfile::tempdir().expect("dir");
        let touch = |d: &Path, n: &str| std::fs::write(d.join(n), "").expect("write");
        assert_eq!(find_lldb_in(&[a.path().to_path_buf()]), None);
        touch(a.path(), "lldb-dap-17");
        touch(b.path(), "lldb-dap-18");
        let both = [a.path().to_path_buf(), b.path().to_path_buf()];
        assert_eq!(find_lldb_in(&both), Some(b.path().join("lldb-dap-18")));
        touch(a.path(), "lldb-vscode");
        assert_eq!(find_lldb_in(&both), Some(a.path().join("lldb-vscode")));
        touch(b.path(), "lldb-dap");
        assert_eq!(
            find_lldb_in(&[b.path().to_path_buf()]),
            Some(b.path().join("lldb-dap"))
        );
    }

    #[test]
    fn a_missing_debugger_says_what_to_run() {
        assert_eq!(
            Install::XcodeTools.command().as_deref(),
            Some("xcode-select --install")
        );
        assert!(install_text(&Install::XcodeToolsOld).contains("brew install llvm"));
        assert_eq!(Install::Unsupported.command(), None);
        let e = Missing {
            language: "rust".into(),
            install: Install::Command("sudo apt install lldb".into()),
        };
        assert_eq!(
            e.to_string(),
            "no debugger found for rust: run: sudo apt install lldb"
        );
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
