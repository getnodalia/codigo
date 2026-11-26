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
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Why there is nothing to debug yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prepare {
    /// Nothing was recognised and nothing is saved: the person has to say which program. The
    /// text says why when there is something to say, e.g. a saved program that is gone.
    Unknown(String),
    /// Something was found but could not be made ready (the build failed).
    Failed(String),
}

impl std::fmt::Display for Prepare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Prepare::Unknown(m) | Prepare::Failed(m) => f.write_str(m),
        }
    }
}

/// The folder where Código keeps what it was told about a project.
const CONFIG_DIR: &str = ".codigo";
const CONFIG_FILE: &str = "debug.json";
const MAX_CONFIG: u64 = 256 * 1024;

/// Languages a saved configuration may name, and the adapter family each uses.
const LANGUAGES: [&str; 5] = ["rust", "c", "cpp", "python", "go"];

/// What the person saved for this project: the program to debug, its arguments, and optionally
/// a command that builds it first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub language: String,
    pub program: String,
    pub args: Vec<String>,
    pub build: Option<String>,
}

/// The language to debug `program` as, from its name: scripts and Go sources are themselves,
/// anything else is a native executable.
pub fn language_of(program: &str) -> &'static str {
    let ext = Path::new(program)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("py") => "python",
        Some("go") => "go",
        _ => "c",
    }
}

/// Read `.codigo/debug.json`.
pub fn saved(root: &Path) -> Option<Saved> {
    let path = root.join(CONFIG_DIR).join(CONFIG_FILE);
    if std::fs::metadata(&path).ok()?.len() > MAX_CONFIG {
        return None;
    }
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let program = v.get("program")?.as_str()?.trim().to_owned();
    if program.is_empty() {
        return None;
    }
    let language = v
        .get("language")
        .and_then(Value::as_str)
        .filter(|l| LANGUAGES.contains(l))
        .map_or_else(|| language_of(&program).to_owned(), str::to_owned);
    let args = v
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let build = v
        .get("build")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_owned);
    Some(Saved {
        language,
        program,
        args,
        build,
    })
}

/// Remember what to debug in this project.
pub fn save(root: &Path, saved: &Saved) -> Result<(), String> {
    if saved.program.trim().is_empty() {
        return Err("the program is empty".into());
    }
    let dir = root.join(CONFIG_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let mut value = serde_json::json!({
        "language": saved.language,
        "program": saved.program.trim(),
        "args": saved.args,
    });
    if let (Some(build), Some(map)) = (&saved.build, value.as_object_mut()) {
        map.insert("build".into(), build.clone().into());
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    let path = dir.join(CONFIG_FILE);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n")
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("could not save {}: {e}", path.display()))
}

/// Split a line of arguments the way a shell would for simple cases: spaces separate, single
/// and double quotes group.
pub fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Work out what to debug in `root`, building it when the language needs that. `active` is the
/// file in the editor: it picks the script of a Python project, the binary of a Rust package
/// with several, and is what gets debugged when the folder is not a project at all. Blocks
/// while building, so call it off the UI thread.
pub fn prepare(root: &Path, active: Option<&Path>) -> Result<Target, Prepare> {
    if let Some(saved) = saved(root) {
        return from_saved(root, &saved);
    }
    if root.join("Cargo.toml").is_file() {
        let program = build_rust(root, active).map_err(Prepare::Failed)?;
        return Ok(Target {
            language: "rust",
            program,
            args: Vec::new(),
            cwd: root.to_path_buf(),
        });
    }
    if root.join("go.mod").is_file() {
        return Ok(Target {
            language: "go",
            program: root.to_path_buf(),
            args: Vec::new(),
            cwd: root.to_path_buf(),
        });
    }
    if let Some(file) = active {
        if let Some(target) = single_file(root, file)? {
            return Ok(target);
        }
    }
    Err(Prepare::Unknown(String::new()))
}

/// A lone source file: scripts run as they are, compiled languages are built with debug
/// information into the temporary folder.
fn single_file(root: &Path, file: &Path) -> Result<Option<Target>, Prepare> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let (language, compiler, extra): (&'static str, &str, &[&str]) = match ext.as_deref() {
        Some("py") => {
            return Ok(Some(Target {
                language: "python",
                program: file.to_path_buf(),
                args: Vec::new(),
                cwd: root.to_path_buf(),
            }))
        }
        Some("go") => {
            return Ok(Some(Target {
                language: "go",
                program: file.to_path_buf(),
                args: Vec::new(),
                cwd: root.to_path_buf(),
            }))
        }
        Some("rs") => ("rust", "rustc", &["--edition", "2021", "-g"][..]),
        Some("c") => ("c", "cc", &["-g", "-O0"][..]),
        Some("cc" | "cpp" | "cxx") => ("cpp", "c++", &["-g", "-O0"][..]),
        _ => return Ok(None),
    };
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    let out = std::env::temp_dir().join(format!("codigo-debug-{stem}"));
    let result = Command::new(compiler)
        .args(extra)
        .arg(file)
        .arg("-o")
        .arg(&out)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Prepare::Failed(format!("could not run {compiler}: {e}")))?;
    if !result.status.success() {
        return Err(Prepare::Failed(format!(
            "the build failed:\n{}",
            tail(&String::from_utf8_lossy(&result.stderr))
        )));
    }
    Ok(Some(Target {
        language,
        program: out,
        args: Vec::new(),
        cwd: root.to_path_buf(),
    }))
}

fn from_saved(root: &Path, saved: &Saved) -> Result<Target, Prepare> {
    if let Some(build) = &saved.build {
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let out = Command::new(shell)
            .args([flag, build])
            .current_dir(root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| Prepare::Failed(format!("could not run `{build}`: {e}")))?;
        if !out.status.success() {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            return Err(Prepare::Failed(format!(
                "the build failed:\n{}",
                tail(&text)
            )));
        }
    }
    let program = root.join(&saved.program);
    if !program.exists() {
        return Err(Prepare::Unknown(format!(
            "{} does not exist; say which program to debug",
            program.display()
        )));
    }
    let language = LANGUAGES
        .iter()
        .find(|l| **l == saved.language)
        .copied()
        .unwrap_or("c");
    Ok(Target {
        language,
        program,
        args: saved.args.clone(),
        cwd: root.to_path_buf(),
    })
}

/// The last lines of some compiler output.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().rev().take(12).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
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
        return Err(format!(
            "the build failed:\n{}",
            tail(&String::from_utf8_lossy(&out.stderr))
        ));
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
        assert!(matches!(prepare(d.path(), None), Err(Prepare::Unknown(_))));
        let py = d.path().join("a.py");
        let t = prepare(d.path(), Some(&py)).expect("python");
        assert_eq!(t.language, "python");
        std::fs::write(d.path().join("go.mod"), "module x").expect("go.mod");
        assert_eq!(prepare(d.path(), None).expect("go").language, "go");
    }

    #[test]
    fn a_saved_program_wins_and_must_exist() {
        let d = tempfile::tempdir().expect("tempdir");
        std::fs::write(d.path().join("go.mod"), "module x").expect("go.mod");
        let cfg = Saved {
            language: "c".into(),
            program: "build/app".into(),
            args: vec!["--fast".into()],
            build: None,
        };
        save(d.path(), &cfg).expect("save");
        assert_eq!(saved(d.path()), Some(cfg));
        // The program is not there yet: ask rather than fail.
        assert!(matches!(prepare(d.path(), None), Err(Prepare::Unknown(_))));
        std::fs::create_dir_all(d.path().join("build")).expect("dir");
        std::fs::write(d.path().join("build/app"), "").expect("app");
        let t = prepare(d.path(), None).expect("saved target");
        assert_eq!((t.language, t.args), ("c", vec!["--fast".to_owned()]));
        assert!(t.program.ends_with("build/app"));
    }

    #[test]
    fn a_failing_build_command_reports_its_output() {
        if cfg!(windows) {
            return;
        }
        let d = tempfile::tempdir().expect("tempdir");
        let cfg = Saved {
            language: "c".into(),
            program: "x".into(),
            args: Vec::new(),
            build: Some("echo nope >&2; exit 3".into()),
        };
        save(d.path(), &cfg).expect("save");
        match prepare(d.path(), None) {
            Err(Prepare::Failed(m)) => assert!(m.contains("nope"), "{m}"),
            other => panic!("expected a failed build, got {other:?}"),
        }
    }

    #[test]
    fn a_broken_or_empty_config_is_ignored() {
        let d = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(d.path().join(CONFIG_DIR)).expect("dir");
        std::fs::write(d.path().join(CONFIG_DIR).join(CONFIG_FILE), "{").expect("write");
        assert_eq!(saved(d.path()), None);
        assert!(save(
            d.path(),
            &Saved {
                language: "c".into(),
                program: "  ".into(),
                args: Vec::new(),
                build: None,
            }
        )
        .is_err());
    }

    #[test]
    fn programs_are_debugged_as_their_language_and_args_split_like_a_shell() {
        assert_eq!(language_of("run.py"), "python");
        assert_eq!(language_of("main.go"), "go");
        assert_eq!(language_of("build/app"), "c");
        assert_eq!(
            split_args(r#"--name "a b" 'c d' e"#),
            vec!["--name", "a b", "c d", "e"]
        );
        assert_eq!(split_args("  "), Vec::<String>::new());
        assert_eq!(split_args(r#""""#), vec![""]);
    }
}
