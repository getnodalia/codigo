// SPDX-License-Identifier: MIT OR Apache-2.0
//! Analysing a whole folder and summing it up.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::duplication::{self, Source};
use crate::rules::{Finding, Severity};
use crate::{analyze_text, MAX_FILE_BYTES};

/// Files analysed per project at most.
const MAX_FILES: usize = 20_000;
/// Findings kept at most; the worst come first.
const MAX_FINDINGS: usize = 5_000;
/// Languages where copied blocks mean copied code (data and markup repeat by nature).
const CODE: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "tsx",
    "c",
    "cpp",
    "go",
    "java",
    "bash",
    "csharp",
    "kotlin",
    "swift",
    "dart",
    "scala",
    "php",
    "ruby",
    "zig",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFinding {
    /// Relative to the analysed folder.
    pub path: PathBuf,
    pub finding: Finding,
}

/// What the quality gate says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Passed,
    /// Nothing critical, but major findings remain.
    Warning,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Worst first, then by file and line.
    pub findings: Vec<ProjectFinding>,
    pub files: usize,
    pub lines: usize,
    /// More was found than is kept, or the analysis was cancelled.
    pub truncated: bool,
}

impl Report {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.finding.severity == severity)
            .count()
    }

    /// Failed on any blocker or critical finding, warns on major ones.
    pub fn gate(&self) -> Gate {
        if self.count(Severity::Blocker) + self.count(Severity::Critical) > 0 {
            Gate::Failed
        } else if self.count(Severity::Major) > 0 {
            Gate::Warning
        } else {
            Gate::Passed
        }
    }
}

fn skipped(path: &str) -> bool {
    let p = path.replace('\\', "/");
    p.ends_with(".lock")
        || p.ends_with(".min.js")
        || p.ends_with(".min.css")
        || p.ends_with("package-lock.json")
        || p.split('/')
            .any(|c| matches!(c, "node_modules" | "vendor" | "dist" | "target" | "build"))
}

/// Analyse every file under `root` that ignore rules do not hide. Binary and big files are
/// skipped. Stops early, with `truncated` set, when `cancel` is.
pub fn analyze_project(root: &Path, cancel: &AtomicBool) -> Report {
    let mut report = Report::default();
    let files = codigo_workspace::list_files(root, MAX_FILES, cancel);
    let mut loaded: Vec<(PathBuf, String, &'static codigo_syntax::LanguageDef)> = Vec::new();
    for rel in files {
        if cancel.load(Ordering::Relaxed) {
            report.truncated = true;
            break;
        }
        let name = rel.to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        let Some(text) = read_text(&root.join(&rel)) else {
            continue;
        };
        let file_name = rel
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let lang =
            codigo_syntax::detect_with_first_line(&file_name, text.lines().next().unwrap_or(""));
        report.files += 1;
        report.lines += text.lines().count();
        for finding in analyze_text(lang, &name, &text) {
            report.findings.push(ProjectFinding {
                path: rel.clone(),
                finding,
            });
        }
        if CODE.contains(&lang.id) {
            loaded.push((rel, text, lang));
        }
    }
    if !report.truncated {
        let names: Vec<String> = loaded
            .iter()
            .map(|(p, _, _)| p.to_string_lossy().into_owned())
            .collect();
        let sources: Vec<Source> = loaded
            .iter()
            .zip(&names)
            .map(|((_, text, _), name)| Source { path: name, text })
            .collect();
        for (file, finding) in duplication::find(&sources) {
            if let Some((path, _, _)) = loaded.get(file) {
                report.findings.push(ProjectFinding {
                    path: path.clone(),
                    finding,
                });
            }
        }
    }
    report.findings.sort_by(|a, b| {
        b.finding
            .severity
            .cmp(&a.finding.severity)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.finding.line.cmp(&b.finding.line))
    });
    if report.findings.len() > MAX_FINDINGS {
        report.findings.truncate(MAX_FINDINGS);
        report.truncated = true;
    }
    report
}

fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_FILE_BYTES || bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::create_dir_all(p.join("src")).expect("src");
        fs::create_dir_all(p.join("target")).expect("target");
        fs::write(p.join(".gitignore"), "target/\n").expect("ignore");
        fs::write(
            p.join("src/a.py"),
            "eval(x)\npassword = \"Zq8f2Lm9Xw4Kp7\"\n",
        )
        .expect("a");
        fs::write(p.join("src/b.py"), "# TODO later\nprint(1)\n").expect("b");
        fs::write(p.join("target/c.py"), "eval(x)\n").expect("c");
        fs::write(p.join("src/bin.dat"), [0u8, 1, 2, 0]).expect("bin");
        d
    }

    #[test]
    fn the_project_is_walked_summed_and_sorted_worst_first() {
        let d = project();
        let report = analyze_project(d.path(), &AtomicBool::new(false));
        assert_eq!(report.files, 3, "a.py, b.py and .gitignore");
        let ids: Vec<_> = report.findings.iter().map(|f| f.finding.rule.id).collect();
        assert_eq!(ids, ["eval", "hardcoded-secret", "todo-comment"]);
        assert!(report
            .findings
            .iter()
            .all(|f| !f.path.starts_with("target")));
        assert_eq!(report.gate(), Gate::Failed);
        assert_eq!(report.count(Severity::Info), 1);
    }

    #[test]
    fn a_clean_project_passes_and_a_cancelled_one_says_so() {
        let d = tempfile::tempdir().expect("tempdir");
        fs::write(d.path().join("a.py"), "print(1)\n").expect("a");
        let report = analyze_project(d.path(), &AtomicBool::new(false));
        assert_eq!(report.gate(), Gate::Passed);
        let cancelled = analyze_project(d.path(), &AtomicBool::new(true));
        assert_eq!(cancelled.files, 0);
    }

    #[test]
    fn copied_code_across_files_is_found() {
        let d = tempfile::tempdir().expect("tempdir");
        let block: String = (0..10)
            .map(|i| format!("    total_{i} = compute({i}) + other_{i}\n"))
            .collect();
        fs::write(d.path().join("a.py"), format!("def a():\n{block}")).expect("a");
        fs::write(d.path().join("b.py"), format!("def b():\n{block}")).expect("b");
        let report = analyze_project(d.path(), &AtomicBool::new(false));
        let dup: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.finding.rule.id == "duplicated-block")
            .collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].path, PathBuf::from("b.py"));
    }
}
