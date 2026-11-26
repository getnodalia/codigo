// SPDX-License-Identifier: MIT OR Apache-2.0
//! The tasks a project offers: how to run, build and test it, found from the files in its folder.

use std::path::Path;

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

impl Task {
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

/// Tasks for the project at `root`, most useful first. An empty list means nothing was
/// recognised; the terminal is always there for the rest.
pub fn detect(root: &Path) -> Vec<Task> {
    use TaskKind::*;
    let mut tasks = Vec::new();
    if root.join("Cargo.toml").is_file() {
        tasks.push(Task::new(Run, "cargo", &["run"]));
        tasks.push(Task::new(Build, "cargo", &["build"]));
        tasks.push(Task::new(Test, "cargo", &["test"]));
        tasks.push(Task::new(
            Check,
            "cargo",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
        ));
    }
    if let Some(scripts) = npm_scripts(root) {
        let runner = if root.join("pnpm-lock.yaml").is_file() {
            "pnpm"
        } else if root.join("yarn.lock").is_file() {
            "yarn"
        } else {
            "npm"
        };
        for (kind, names) in [
            (Run, &["start", "dev"][..]),
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
            }
        }
    }
    if root.join("go.mod").is_file() {
        tasks.push(Task::new(Run, "go", &["run", "."]));
        tasks.push(Task::new(Build, "go", &["build", "./..."]));
        tasks.push(Task::new(Test, "go", &["test", "./..."]));
        tasks.push(Task::new(Check, "go", &["vet", "./..."]));
    }
    let python = root.join("main.py").is_file();
    if python {
        tasks.push(Task::new(Run, "python3", &["main.py"]));
    }
    if root.join("pyproject.toml").is_file() || root.join("pytest.ini").is_file() {
        tasks.push(Task::new(Test, "python3", &["-m", "pytest"]));
    }
    if root.join("Makefile").is_file() || root.join("makefile").is_file() {
        tasks.push(Task::new(Build, "make", &[]));
        tasks.push(Task::new(Test, "make", &["test"]));
    }
    tasks
}

/// The first task of a kind.
pub fn pick(tasks: &[Task], kind: TaskKind) -> Option<&Task> {
    tasks.iter().find(|t| t.kind == kind)
}

fn npm_scripts(root: &Path) -> Option<Vec<String>> {
    let path = root.join("package.json");
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_MANIFEST {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(value.get("scripts")?.as_object()?.keys().cloned().collect())
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
        let dir = project(&[("go.mod", "module x"), ("main.py", ""), ("Makefile", "")]);
        let labels: Vec<_> = detect(dir.path()).into_iter().map(|t| t.label).collect();
        assert!(labels.contains(&"go run .".to_owned()));
        assert!(labels.contains(&"python3 main.py".to_owned()));
        assert!(labels.contains(&"make test".to_owned()));
    }
}
