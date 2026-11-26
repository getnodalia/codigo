// SPDX-License-Identifier: MIT OR Apache-2.0
//! What to debug in a folder, and building it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// A program ready to be debugged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The language id (as in `codigo-syntax`), which picks the adapter.
    pub language: &'static str,
    /// What the adapter launches: the built executable, the script, or the Go package folder.
    pub program: PathBuf,
    pub cwd: PathBuf,
}

/// Work out what to debug in `root`, building it when the language needs that. `active` is the
/// file in the editor: it picks the script of a Python project and the binary of a Rust package
/// with several. Blocks while building, so call it off the UI thread.
pub fn prepare(root: &Path, active: Option<&Path>) -> Result<Target, String> {
    if root.join("Cargo.toml").is_file() {
        let program = build_rust(root, active)?;
        return Ok(Target {
            language: "rust",
            program,
            cwd: root.to_path_buf(),
        });
    }
    if root.join("go.mod").is_file() {
        return Ok(Target {
            language: "go",
            program: root.to_path_buf(),
            cwd: root.to_path_buf(),
        });
    }
    if let Some(file) = active.filter(|f| f.extension().is_some_and(|e| e == "py")) {
        return Ok(Target {
            language: "python",
            program: file.to_path_buf(),
            cwd: root.to_path_buf(),
        });
    }
    Err(
        "Código can debug Rust and Go projects, and a Python file open in the editor, so far"
            .into(),
    )
}

/// `cargo build` and the executable it made.
fn build_rust(root: &Path, active: Option<&Path>) -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["build", "--message-format=json-render-diagnostics"])
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(12).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("the build failed:\n{}", tail.join("\n")));
    }
    let executables = parse_executables(&String::from_utf8_lossy(&out.stdout));
    pick(&executables, active).ok_or_else(|| {
        "the project built but makes no program to debug (it has no binary target)".into()
    })
}

/// A binary the build made, and the source file it starts from.
#[derive(Debug, PartialEq, Eq)]
struct Built {
    executable: PathBuf,
    source: PathBuf,
}

/// The binaries in `cargo build --message-format=json` output.
fn parse_executables(json_lines: &str) -> Vec<Built> {
    let mut out = Vec::new();
    for line in json_lines.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let is_bin = v
            .get("target")
            .and_then(|t| t.get("kind"))
            .and_then(Value::as_array)
            .is_some_and(|k| k.iter().any(|k| k == "bin"));
        let (Some(exe), true) = (v.get("executable").and_then(Value::as_str), is_bin) else {
            continue;
        };
        let source = v
            .get("target")
            .and_then(|t| t.get("src_path"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        out.push(Built {
            executable: PathBuf::from(exe),
            source: PathBuf::from(source),
        });
    }
    out
}

/// The binary whose source is the open file, else the first.
fn pick(built: &[Built], active: Option<&Path>) -> Option<PathBuf> {
    active
        .and_then(|a| built.iter().find(|b| b.source == a))
        .or_else(|| built.first())
        .map(|b| b.executable.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = r#"
{"reason":"compiler-artifact","executable":null,"target":{"kind":["lib"],"src_path":"/p/src/lib.rs"}}
{"reason":"compiler-artifact","executable":"/p/target/debug/a","target":{"kind":["bin"],"src_path":"/p/src/main.rs"}}
not json
{"reason":"compiler-artifact","executable":"/p/target/debug/b","target":{"kind":["bin"],"src_path":"/p/src/bin/b.rs"}}
{"reason":"compiler-artifact","executable":"/p/target/debug/t","target":{"kind":["test"],"src_path":"/p/tests/t.rs"}}
{"reason":"build-finished","success":true}
"#;

    #[test]
    fn only_binaries_are_taken_and_the_open_file_picks_one() {
        let built = parse_executables(OUT);
        assert_eq!(built.len(), 2);
        assert_eq!(pick(&built, None), Some(PathBuf::from("/p/target/debug/a")));
        assert_eq!(
            pick(&built, Some(Path::new("/p/src/bin/b.rs"))),
            Some(PathBuf::from("/p/target/debug/b"))
        );
        assert_eq!(
            pick(&built, Some(Path::new("/elsewhere.rs"))),
            Some(PathBuf::from("/p/target/debug/a"))
        );
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn the_folder_decides_the_language() {
        let d = tempfile::tempdir().expect("tempdir");
        let err = prepare(d.path(), None).expect_err("nothing to debug");
        assert!(err.contains("Rust"));
        let py = d.path().join("a.py");
        let t = prepare(d.path(), Some(&py)).expect("python");
        assert_eq!(t.language, "python");
        std::fs::write(d.path().join("go.mod"), "module x").expect("go.mod");
        assert_eq!(prepare(d.path(), None).expect("go").language, "go");
    }
}
