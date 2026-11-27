// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the assistant is told about the open project: the file in the editor, the selection, the
//! other open files, an outline of the folder, and the files the person mentioned with `@`.
//!
//! This is built fresh for every question, so the answer is about the code as it is now. Every
//! part is capped, and the person can turn the whole thing off in Preferences.

use crate::prompts::{clip, fenced};

/// The most of the open file that goes along.
pub const MAX_ACTIVE: usize = 40_000;
/// The most of one mentioned file that goes along.
pub const MAX_ATTACHED: usize = 20_000;
/// The most mentioned files in one question.
pub const MAX_MENTIONS: usize = 5;
/// The most paths in the folder outline.
pub const MAX_OUTLINE: usize = 250;

/// The file in the editor.
#[derive(Clone, Debug, Default)]
pub struct Active {
    /// Relative to the project folder when it is inside it.
    pub path: String,
    pub language: String,
    pub text: String,
    /// 1-based line of the caret.
    pub cursor_line: usize,
    /// The selected text, when there is one, with the 1-based line it starts on.
    pub selection: Option<(usize, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    pub project: Option<String>,
    /// Relative paths of the project's files.
    pub files: Vec<String>,
    /// Names of the open tabs.
    pub open: Vec<String>,
    pub active: Option<Active>,
    /// Files the person mentioned: path and text.
    pub attached: Vec<(String, String)>,
}

/// Around line `cursor` of `text`, at most `max` bytes, cut at line boundaries. The first number
/// is the 1-based line the excerpt starts on; the second says whether anything was left out.
pub fn window(text: &str, cursor: usize, max: usize) -> (usize, String, bool) {
    if text.len() <= max {
        return (1, text.to_owned(), false);
    }
    let lines: Vec<&str> = text.lines().collect();
    let at = cursor.saturating_sub(1).min(lines.len().saturating_sub(1));
    let (mut lo, mut hi) = (at, at + 1);
    let mut used = lines.get(at).map_or(0, |l| l.len() + 1);
    loop {
        let mut grew = false;
        if lo > 0 {
            let add = lines.get(lo - 1).map_or(0, |l| l.len() + 1);
            if used + add <= max {
                lo -= 1;
                used += add;
                grew = true;
            }
        }
        if hi < lines.len() {
            let add = lines.get(hi).map_or(0, |l| l.len() + 1);
            if used + add <= max {
                hi += 1;
                used += add;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let excerpt = lines.get(lo..hi).unwrap_or(&[]).join("\n");
    // A single line longer than the cap still has to fit.
    (lo + 1, clip(&excerpt, max).to_owned(), true)
}

impl Context {
    /// The text that goes after the instructions, empty when there is nothing to say.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if let Some(name) = &self.project {
            out.push_str(&format!("Project folder: {name}\n"));
        }
        if !self.files.is_empty() {
            out.push_str("Files in the project");
            if self.files.len() > MAX_OUTLINE {
                out.push_str(&format!(
                    " (the first {MAX_OUTLINE} of {})",
                    self.files.len()
                ));
            }
            out.push_str(":\n");
            for f in self.files.iter().take(MAX_OUTLINE) {
                out.push_str(f);
                out.push('\n');
            }
        }
        if !self.open.is_empty() {
            out.push_str(&format!("\nOpen tabs: {}\n", self.open.join(", ")));
        }
        if let Some(a) = &self.active {
            out.push_str(&format!(
                "\nThe file in the editor is {} ({}), the caret is on line {}.\n",
                a.path,
                if a.language.is_empty() {
                    "plain text"
                } else {
                    &a.language
                },
                a.cursor_line.max(1)
            ));
            let (first, text, cut) = window(&a.text, a.cursor_line, MAX_ACTIVE);
            if cut {
                out.push_str(&format!(
                    "It is long, so only part of it follows, starting at line {first}.\n"
                ));
            }
            out.push_str(&fenced(&a.language, &text));
            out.push('\n');
            if let Some((line, sel)) = &a.selection {
                out.push_str(&format!(
                    "\nThe person has selected this, starting at line {line}:\n{}\n",
                    fenced(&a.language, sel)
                ));
            }
        }
        for (path, text) in &self.attached {
            out.push_str(&format!(
                "\nThe person mentioned the file {path}:\n{}\n",
                fenced("", clip(text, MAX_ATTACHED))
            ));
        }
        out
    }
}

/// The files `@name` points at in `input`: a path in `files`, or a bare file name that only one
/// path ends with. At most [`MAX_MENTIONS`], each once.
pub fn mentions(input: &str, files: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in input.split_whitespace() {
        let Some(name) = word.strip_prefix('@') else {
            continue;
        };
        let name = name.trim_end_matches([',', ';', ':', ')', '?', '!']);
        let name = name.trim_end_matches('.');
        if name.is_empty() {
            continue;
        }
        let exact = files.iter().find(|f| f.as_str() == name);
        let by_name = || {
            let tail = format!("/{name}");
            let mut hits = files.iter().filter(|f| f.ends_with(&tail));
            match (hits.next(), hits.next()) {
                (Some(one), None) => Some(one),
                _ => None,
            }
        };
        if let Some(path) = exact.or_else(by_name) {
            if !found.contains(path) {
                found.push(path.clone());
            }
        }
        if found.len() >= MAX_MENTIONS {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_file_goes_whole_and_a_long_one_around_the_caret() {
        let (first, text, cut) = window("a\nb\nc", 2, 100);
        assert_eq!((first, text.as_str(), cut), (1, "a\nb\nc", false));
        let long: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        let (first, text, cut) = window(&long, 500, 400);
        assert!(cut);
        assert!(text.len() <= 400);
        assert!(first > 400 && first < 500, "{first}");
        assert!(text.contains("line 500"));
    }

    #[test]
    fn what_the_assistant_is_told_names_the_file_the_caret_and_the_selection() {
        let ctx = Context {
            project: Some("demo".into()),
            files: vec!["src/main.rs".into(), "Cargo.toml".into()],
            open: vec!["main.rs".into()],
            active: Some(Active {
                path: "src/main.rs".into(),
                language: "rust".into(),
                text: "fn main() {}\n".into(),
                cursor_line: 1,
                selection: Some((1, "main".into())),
            }),
            attached: vec![("Cargo.toml".into(), "[package]".into())],
        };
        let text = ctx.render();
        for needle in [
            "Project folder: demo",
            "src/main.rs",
            "caret is on line 1",
            "fn main() {}",
            "selected this, starting at line 1",
            "mentioned the file Cargo.toml",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        assert!(Context::default().render().is_empty());
    }

    #[test]
    fn an_outline_of_a_huge_project_is_cut() {
        let ctx = Context {
            files: (0..1000).map(|n| format!("f{n}.rs")).collect(),
            ..Context::default()
        };
        let text = ctx.render();
        assert!(text.contains("the first 250 of 1000"));
        assert!(!text.contains("f999.rs"));
    }

    #[test]
    fn mentions_find_paths_and_unique_names_only() {
        let files: Vec<String> = ["src/a.rs", "src/b.rs", "docs/a.rs", "Cargo.toml"]
            .map(String::from)
            .into();
        assert_eq!(
            mentions("look at @Cargo.toml, and @src/b.rs.", &files),
            vec!["Cargo.toml", "src/b.rs"]
        );
        // `a.rs` is in two folders: no guessing.
        assert!(mentions("@a.rs", &files).is_empty());
        assert!(mentions("@nothing @ @", &files).is_empty());
        assert_eq!(mentions("@b.rs @b.rs", &files), vec!["src/b.rs"]);
    }
}
