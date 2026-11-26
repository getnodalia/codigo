// SPDX-License-Identifier: MIT OR Apache-2.0
//! The tasks a project offers: how to run, build and test it, found from the files in its folder.

use std::path::{Path, PathBuf};

/// What a task is for. Commands pick the first task of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Run,
    Build,
    Test,
    /// Linting and formatting checks.
    Check,
}

/// A command line to run in the project folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub kind: TaskKind,
    /// What the person sees, e.g. `cargo test`.
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
}

impl TaskKind {
    /// The key of this kind in the project's saved tasks.
    pub fn key(self) -> &'static str {
        match self {
            TaskKind::Run => "run",
            TaskKind::Build => "build",
            TaskKind::Test => "test",
            TaskKind::Check => "check",
        }
    }

    pub const ALL: [TaskKind; 4] = [
        TaskKind::Run,
        TaskKind::Build,
        TaskKind::Test,
        TaskKind::Check,
    ];
}

impl Task {
    /// A command line typed by a person, run through the system shell so pipes, `&&` and
    /// environment variables work.
    pub fn shell(kind: TaskKind, command: &str) -> Task {
        let command = command.trim();
        let (program, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let mut label: String = command.chars().take(48).collect();
        if command.chars().count() > 48 {
            label.push('…');
        }
        Task {
            kind,
            label,
            program: program.to_owned(),
            args: vec![flag.to_owned(), command.to_owned()],
        }
    }

    fn new(kind: TaskKind, program: &str, args: &[&str]) -> Task {
        let mut label = program.to_owned();
        for a in args {
            label.push(' ');
            label.push_str(a);
        }
        Task {
            kind,
            label,
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// The largest package.json read for its scripts.
const MAX_MANIFEST: u64 = 1024 * 1024;

/// The largest Makefile read for its targets.
const MAX_MAKEFILE: u64 = 256 * 1024;

/// The folder where Código keeps what it was told about a project.
pub const CONFIG_DIR: &str = ".codigo";
const TASKS_FILE: &str = "tasks.json";

/// Tasks for the project at `root`, most useful first: the ones the person saved, then the ones
/// found from the project's files. An empty list means nothing was recognised.
pub fn detect(root: &Path) -> Vec<Task> {
    let mut tasks = saved(root);
    tasks.extend(detect_files(root));
    tasks
}

/// What to run: the project's task of this kind, and for `Run` the file in the editor when the
/// project has none (a lone script runs as it is). `None` means the person has to say.
pub fn resolve(root: &Path, active: Option<&Path>, kind: TaskKind) -> Option<Task> {
    let tasks = detect(root);
    if let Some(task) = pick(&tasks, kind) {
        return Some(task.clone());
    }
    if kind == TaskKind::Run {
        return active.and_then(for_file);
    }
    None
}

/// The tasks the person saved for this project in `.codigo/tasks.json`.
pub fn saved(root: &Path) -> Vec<Task> {
    let Some(value) = read_json(&root.join(CONFIG_DIR).join(TASKS_FILE)) else {
        return Vec::new();
    };
    TaskKind::ALL
        .iter()
        .filter_map(|kind| {
            let command = value.get(kind.key())?.as_str()?.trim();
            (!command.is_empty()).then(|| Task::shell(*kind, command))
        })
        .collect()
}

/// Remember how to do `kind` in this project. Other saved kinds are kept.
pub fn save(root: &Path, kind: TaskKind, command: &str) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("the command is empty".into());
    }
    let dir = root.join(CONFIG_DIR);
    let path = dir.join(TASKS_FILE);
    let mut value = read_json(&path)
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(map) = value.as_object_mut() {
        map.insert(kind.key().to_owned(), command.into());
    }
    write_json(&dir, &path, &value)
}

/// Read a small JSON file, or nothing when it is missing, huge or not JSON.
pub fn read_json(path: &Path) -> Option<serde_json::Value> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_MANIFEST {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Write JSON next to its final name first and rename, so a crash never leaves half a file.
pub fn write_json(dir: &Path, path: &Path, value: &serde_json::Value) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n")
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("could not save {}: {e}", path.display()))
}

/// Single-quote `text` for the shell.
fn quote(text: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", text.replace('"', ""))
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// How to run one file by itself, from its extension. Compiled languages build into the
/// temporary folder and run from there.
pub fn for_file(path: &Path) -> Option<Task> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let file = quote(&path.to_string_lossy());
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let out = std::env::temp_dir().join(format!("codigo-run-{stem}"));
    let out = quote(&out.to_string_lossy());
    let command = match ext.as_str() {
        "py" => format!("python3 -u {file}"),
        "js" | "mjs" | "cjs" => format!("node {file}"),
        "ts" | "mts" | "tsx" => format!("npx --yes tsx {file}"),
        "rb" => format!("ruby {file}"),
        "php" => format!("php {file}"),
        "lua" => format!("lua {file}"),
        "pl" => format!("perl {file}"),
        "sh" | "bash" => format!("bash {file}"),
        "ps1" => format!("pwsh -File {file}"),
        "go" => format!("go run {file}"),
        "java" => format!("java {file}"),
        "swift" => format!("swift {file}"),
        "dart" => format!("dart run {file}"),
        "ex" | "exs" => format!("elixir {file}"),
        "jl" => format!("julia {file}"),
        "r" => format!("Rscript {file}"),
        "hs" => format!("runghc {file}"),
        "zig" => format!("zig run {file}"),
        "rs" => format!("rustc --edition 2021 {file} -o {out} && {out}"),
        "c" => format!("cc {file} -o {out} && {out}"),
        "cc" | "cpp" | "cxx" => format!("c++ {file} -o {out} && {out}"),
        _ => return None,
    };
    Some(Task::shell(TaskKind::Run, &command))
}

fn detect_files(root: &Path) -> Vec<Task> {
    use TaskKind::*;
    let has = |name: &str| root.join(name).is_file();
    let mut tasks = Vec::new();
    if has("Cargo.toml") {
        tasks.push(Task::new(Run, "cargo", &["run"]));
        tasks.push(Task::new(Build, "cargo", &["build"]));
        tasks.push(Task::new(Test, "cargo", &["test"]));
        tasks.push(Task::new(
            Check,
            "cargo",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
        ));
    }
    if let Some(manifest) = npm_manifest(root) {
        let runner = if has("bun.lockb") || has("bun.lock") {
            "bun"
        } else if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else {
            "npm"
        };
        let scripts = &manifest.scripts;
        let mut run_found = false;
        for (kind, names) in [
            (Run, &["start", "dev", "serve"][..]),
            (Build, &["build"][..]),
            (Test, &["test"][..]),
            (Check, &["lint"][..]),
        ] {
            if let Some(name) = names.iter().find(|n| scripts.iter().any(|s| s == *n)) {
                // `npm start` and `npm test` have shortcuts; every other npm script needs `run`.
                let args: Vec<&str> = if runner == "npm" && !matches!(*name, "start" | "test") {
                    vec!["run", name]
                } else {
                    vec![name]
                };
                tasks.push(Task::new(kind, runner, &args));
                run_found |= kind == Run;
            }
        }
        if !run_found {
            if let Some(main) = manifest.main.filter(|m| root.join(m).is_file()) {
                tasks.push(Task::new(Run, "node", &[&main]));
            }
        }
    }
    if has("deno.json") || has("deno.jsonc") {
        for (kind, name) in [
            (Run, "start"),
            (Run, "dev"),
            (Test, "test"),
            (Check, "lint"),
        ] {
            if deno_tasks(root).iter().any(|t| t == name) {
                tasks.push(Task::new(kind, "deno", &["task", name]));
            }
        }
    }
    if has("go.mod") {
        tasks.push(Task::new(Run, "go", &["run", "."]));
        tasks.push(Task::new(Build, "go", &["build", "./..."]));
        tasks.push(Task::new(Test, "go", &["test", "./..."]));
        tasks.push(Task::new(Check, "go", &["vet", "./..."]));
    }
    if has("manage.py") {
        tasks.push(Task::new(Run, "python3", &["manage.py", "runserver"]));
        tasks.push(Task::new(Test, "python3", &["manage.py", "test"]));
    } else if let Some(main) = ["main.py", "app.py", "run.py", "__main__.py"]
        .into_iter()
        .find(|f| has(f))
    {
        tasks.push(Task::new(Run, "python3", &[main]));
    }
    if has("pyproject.toml") || has("pytest.ini") || has("setup.py") {
        tasks.push(Task::new(Test, "python3", &["-m", "pytest"]));
    }
    if has("pom.xml") {
        tasks.push(Task::new(Build, "mvn", &["-q", "package"]));
        tasks.push(Task::new(Test, "mvn", &["-q", "test"]));
    }
    if has("build.gradle") || has("build.gradle.kts") {
        let gradle = if has("gradlew") {
            "./gradlew"
        } else {
            "gradle"
        };
        tasks.push(Task::new(Run, gradle, &["run"]));
        tasks.push(Task::new(Build, gradle, &["build"]));
        tasks.push(Task::new(Test, gradle, &["test"]));
    }
    if has("build.zig") {
        tasks.push(Task::new(Run, "zig", &["build", "run"]));
        tasks.push(Task::new(Build, "zig", &["build"]));
        tasks.push(Task::new(Test, "zig", &["build", "test"]));
    }
    if has("Package.swift") {
        tasks.push(Task::new(Run, "swift", &["run"]));
        tasks.push(Task::new(Build, "swift", &["build"]));
        tasks.push(Task::new(Test, "swift", &["test"]));
    }
    if has("mix.exs") {
        tasks.push(Task::new(Run, "mix", &["run"]));
        tasks.push(Task::new(Build, "mix", &["compile"]));
        tasks.push(Task::new(Test, "mix", &["test"]));
    }
    if has("pubspec.yaml") {
        let flutter = std::fs::read_to_string(root.join("pubspec.yaml"))
            .is_ok_and(|t| t.contains("flutter:"));
        let tool = if flutter { "flutter" } else { "dart" };
        tasks.push(Task::new(Run, tool, &["run"]));
        tasks.push(Task::new(Test, tool, &["test"]));
    }
    if has("Gemfile") {
        if has("main.rb") {
            tasks.push(Task::new(Run, "bundle", &["exec", "ruby", "main.rb"]));
        }
        tasks.push(Task::new(Test, "bundle", &["exec", "rake", "test"]));
    } else if has("main.rb") {
        tasks.push(Task::new(Run, "ruby", &["main.rb"]));
    }
    if has("composer.json") {
        tasks.push(Task::new(Test, "composer", &["test"]));
    }
    if has_with_extension(root, &["sln", "csproj", "fsproj"]) {
        tasks.push(Task::new(Run, "dotnet", &["run"]));
        tasks.push(Task::new(Build, "dotnet", &["build"]));
        tasks.push(Task::new(Test, "dotnet", &["test"]));
    }
    if has("CMakeLists.txt") {
        tasks.push(Task::shell(
            Build,
            "cmake -S . -B build && cmake --build build",
        ));
        tasks.push(Task::shell(
            Test,
            "ctest --test-dir build --output-on-failure",
        ));
    }
    if has("Makefile") || has("makefile") || has("GNUmakefile") {
        let targets = make_targets(root);
        if targets.iter().any(|t| t == "run") {
            tasks.push(Task::new(Run, "make", &["run"]));
        }
        tasks.push(Task::new(Build, "make", &[]));
        if targets.iter().any(|t| t == "test" || t == "check") {
            let name = if targets.iter().any(|t| t == "test") {
                "test"
            } else {
                "check"
            };
            tasks.push(Task::new(Test, "make", &[name]));
        }
    }
    tasks
}

fn has_with_extension(root: &Path, extensions: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().take(500).any(|e| {
        e.path()
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| extensions.contains(&x))
    })
}

/// The targets a Makefile defines at the start of a line.
fn make_targets(root: &Path) -> Vec<String> {
    let path: Option<PathBuf> = ["Makefile", "makefile", "GNUmakefile"]
        .into_iter()
        .map(|n| root.join(n))
        .find(|p| p.is_file());
    let Some(path) = path else {
        return Vec::new();
    };
    if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_MAKEFILE) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.starts_with(['\t', ' ', '#', '.']))
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let name = name.trim();
            let simple = !name.is_empty()
                && !rest.starts_with('=')
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
            simple.then(|| name.to_owned())
        })
        .collect()
}

fn deno_tasks(root: &Path) -> Vec<String> {
    ["deno.json", "deno.jsonc"]
        .into_iter()
        .filter_map(|n| read_json(&root.join(n)))
        .find_map(|v| Some(v.get("tasks")?.as_object()?.keys().cloned().collect()))
        .unwrap_or_default()
}

/// The first task of a kind.
pub fn pick(tasks: &[Task], kind: TaskKind) -> Option<&Task> {
    tasks.iter().find(|t| t.kind == kind)
}

struct Manifest {
    scripts: Vec<String>,
    main: Option<String>,
}

fn npm_manifest(root: &Path) -> Option<Manifest> {
    let value = read_json(&root.join("package.json"))?;
    let scripts = value
        .get("scripts")
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let main = value
        .get("main")
        .and_then(|m| m.as_str())
        .map(str::to_owned);
    Some(Manifest { scripts, main })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("dir");
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).expect("write");
        }
        dir
    }

    #[test]
    fn a_cargo_project_runs_builds_and_tests() {
        let dir = project(&[("Cargo.toml", "[package]")]);
        let tasks = detect(dir.path());
        assert_eq!(
            pick(&tasks, TaskKind::Run).map(|t| t.label.as_str()),
            Some("cargo run")
        );
        assert_eq!(
            pick(&tasks, TaskKind::Test).map(|t| t.label.as_str()),
            Some("cargo test")
        );
        assert!(pick(&tasks, TaskKind::Check).is_some());
    }

    #[test]
    fn npm_scripts_become_tasks() {
        let dir = project(&[(
            "package.json",
            r#"{"scripts":{"dev":"vite","build":"vite build","test":"vitest"}}"#,
        )]);
        let tasks = detect(dir.path());
        assert_eq!(
            pick(&tasks, TaskKind::Run).map(|t| t.label.as_str()),
            Some("npm run dev")
        );
        assert_eq!(
            pick(&tasks, TaskKind::Build).map(|t| t.label.as_str()),
            Some("npm run build")
        );
        assert_eq!(
            pick(&tasks, TaskKind::Test).map(|t| t.label.as_str()),
            Some("npm test")
        );
        assert!(pick(&tasks, TaskKind::Check).is_none());
    }

    #[test]
    fn yarn_and_pnpm_are_chosen_by_their_lock_files() {
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node ."}}"#),
            ("pnpm-lock.yaml", ""),
        ]);
        let tasks = detect(dir.path());
        assert_eq!(
            pick(&tasks, TaskKind::Run).map(|t| t.label.as_str()),
            Some("pnpm start")
        );
    }

    #[test]
    fn a_broken_manifest_gives_no_tasks_and_no_crash() {
        let dir = project(&[("package.json", "{ not json")]);
        assert!(detect(dir.path()).is_empty());
        let empty = project(&[]);
        assert!(detect(empty.path()).is_empty());
    }

    #[test]
    fn go_python_and_make_are_recognised() {
        let dir = project(&[
            ("go.mod", "module x"),
            ("main.py", ""),
            ("Makefile", "test:\n\ttrue\n"),
        ]);
        let labels: Vec<_> = detect(dir.path()).into_iter().map(|t| t.label).collect();
        assert!(labels.contains(&"go run .".to_owned()));
        assert!(labels.contains(&"python3 main.py".to_owned()));
        assert!(labels.contains(&"make test".to_owned()));
    }

    #[test]
    fn saved_tasks_win_and_are_kept_between_saves() {
        let dir = project(&[("Cargo.toml", "[package]")]);
        save(dir.path(), TaskKind::Run, "cargo run --release").expect("save");
        save(dir.path(), TaskKind::Test, "cargo nextest run").expect("save");
        let tasks = detect(dir.path());
        let run = pick(&tasks, TaskKind::Run).expect("run");
        assert_eq!(run.label, "cargo run --release");
        assert_eq!(
            run.args.last().map(String::as_str),
            Some("cargo run --release")
        );
        assert_eq!(
            pick(&tasks, TaskKind::Test).map(|t| t.label.as_str()),
            Some("cargo nextest run")
        );
        // Build still comes from Cargo.toml.
        assert_eq!(
            pick(&tasks, TaskKind::Build).map(|t| t.label.as_str()),
            Some("cargo build")
        );
        assert!(save(dir.path(), TaskKind::Run, "   ").is_err());
    }

    #[test]
    fn a_broken_saved_file_is_ignored_and_replaced() {
        let dir = project(&[]);
        std::fs::create_dir_all(dir.path().join(CONFIG_DIR)).expect("dir");
        std::fs::write(dir.path().join(CONFIG_DIR).join("tasks.json"), "[1,").expect("write");
        assert!(saved(dir.path()).is_empty());
        save(dir.path(), TaskKind::Run, "make go").expect("save");
        assert_eq!(saved(dir.path()).len(), 1);
    }

    #[test]
    fn a_lone_file_runs_by_its_extension() {
        let dir = project(&[]);
        let py = dir.path().join("hello.py");
        let task = resolve(dir.path(), Some(&py), TaskKind::Run).expect("python runs");
        assert!(task.label.starts_with("python3 -u "));
        assert!(for_file(Path::new("a.rs"))
            .expect("rust")
            .label
            .contains("rustc"));
        assert!(for_file(Path::new("notes.txt")).is_none());
        // Only Run falls back to the open file.
        assert!(resolve(dir.path(), Some(&py), TaskKind::Test).is_none());
        // A path with a quote cannot break out of the command.
        let evil = for_file(Path::new("it's.py")).expect("python");
        assert!(
            evil.args
                .last()
                .is_some_and(|c| c.contains("'it'\\''s.py'"))
                || cfg!(windows)
        );
    }

    #[test]
    fn more_ecosystems_are_recognised() {
        let dir = project(&[
            ("build.zig", ""),
            ("mix.exs", ""),
            ("Package.swift", ""),
            ("CMakeLists.txt", ""),
            ("Makefile", "run:\n\t./a.out\ntest: all\n\t./t\nCC := cc\n"),
            ("app.csproj", ""),
        ]);
        let labels: Vec<_> = detect(dir.path()).into_iter().map(|t| t.label).collect();
        for want in [
            "zig build run",
            "mix test",
            "swift run",
            "make run",
            "make test",
            "dotnet run",
        ] {
            assert!(labels.contains(&want.to_owned()), "{want} in {labels:?}");
        }
        assert!(labels.iter().any(|l| l.starts_with("cmake -S")));
        // `CC := cc` is a variable, not a target.
        assert!(!labels.contains(&"make CC".to_owned()));
    }

    #[test]
    fn node_main_runs_when_there_is_no_start_script() {
        let dir = project(&[
            ("package.json", r#"{"main":"server.js","scripts":{}}"#),
            ("server.js", ""),
        ]);
        assert_eq!(
            pick(&detect(dir.path()), TaskKind::Run).map(|t| t.label.as_str()),
            Some("node server.js")
        );
    }
}
