// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's code quality and security analysis, in the spirit of SonarQube but direct: no server
//! to run and nothing leaves the computer. Rules look for hard-coded secrets, unsafe patterns and
//! sloppy shortcuts line by line, measure each function's length, complexity and parameters on
//! the syntax tree, and find blocks of code that were copied. A [`Report`] sums a project up and
//! says whether it passes the quality gate.
//!
//! Nothing here draws anything. Every input is hostile text: sizes are capped and nothing panics.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod duplication;
mod lines;
mod nodalia;
mod project;
mod rules;
mod structure;

pub use project::{analyze_project, Gate, ProjectFinding, Report};
pub use rules::{rule, Finding, Kind, Rule, Severity, RULES};

use codigo_syntax::LanguageDef;

/// Largest file the analysis reads, in bytes. Bigger files are data, not source.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Every finding in one file: the line rules and the structure rules. `path` only decides whether
/// the file is test code (`tests/`, `benches/`), where shortcuts such as `unwrap` are fine.
pub fn analyze_text(lang: &'static LanguageDef, path: &str, text: &str) -> Vec<Finding> {
    analyze_text_with(lang, path, text, false)
}

/// [`analyze_text`], also checking Nodalia's engineering rules (`ndl-…`) when `nodalia` is set:
/// colours from the theme tokens, interface text through the translation functions.
pub fn analyze_text_with(
    lang: &'static LanguageDef,
    path: &str,
    text: &str,
    nodalia: bool,
) -> Vec<Finding> {
    if text.len() > MAX_FILE_BYTES || ignored_file(text) {
        return Vec::new();
    }
    let rope = ropey::Rope::from_str(text);
    let mut syntax = codigo_syntax::Syntax::new(lang);
    if let Some(s) = syntax.as_mut() {
        s.reparse(&rope);
    }
    let tree = syntax.as_ref().and_then(|s| s.tree());
    let mut out = lines::check(lang, path, text, tree, nodalia);
    if let Some(tree) = tree {
        structure::check(tree, text, &mut out);
    }
    out.sort_by_key(|f| (f.line, f.start));
    out
}

/// A file that says `codigo-ignore-file` near its top is skipped: rule tables and test fixtures
/// are full of the very patterns the rules look for.
fn ignored_file(text: &str) -> bool {
    text.lines()
        .take(6)
        .any(|l| l.contains("codigo-ignore-file"))
}
