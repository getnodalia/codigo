// SPDX-License-Identifier: MIT OR Apache-2.0
//! Searching the text of every file in a project.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use codigo_text::search::{Matcher, Query};

use crate::walk::walker;

/// Files bigger than this are skipped; they are data, not source.
const DEFAULT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// A line longer than this is shown cut, and a match beyond the cut is not reported.
const MAX_PREVIEW_BYTES: usize = 400;

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub query: Query,
    pub max_results: usize,
    pub max_file_bytes: u64,
}

impl SearchOptions {
    pub fn new(query: Query) -> Self {
        Self {
            query,
            max_results: 5_000,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

/// One match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Relative to the searched folder.
    pub path: PathBuf,
    /// 0-based.
    pub line: usize,
    /// Byte columns inside `preview`.
    pub start: usize,
    pub end: usize,
    /// The line, without its break, cut at 400 bytes.
    pub preview: String,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    /// More matches existed than `max_results`, or the search was cancelled.
    pub truncated: bool,
    pub files_scanned: usize,
}

/// Search every file under `root` that ignore rules do not hide. Binary files, files too large
/// and files that cannot be read are skipped. The error is the query's, when it does not compile.
pub fn search(
    root: &Path,
    options: &SearchOptions,
    cancel: &AtomicBool,
) -> Result<SearchResult, String> {
    let matcher = Matcher::new(&options.query)?;
    let mut result = SearchResult::default();
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in walker(root).build() {
        if cancel.load(Ordering::Relaxed) {
            result.truncated = true;
            return Ok(result);
        }
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_some_and(|t| t.is_file()) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            result.truncated = true;
            break;
        }
        let Some(text) = read_text(&path, options.max_file_bytes) else {
            continue;
        };
        result.files_scanned += 1;
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        for (line_no, line) in text.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            for (start, end) in matcher.in_text(line) {
                if result.hits.len() >= options.max_results {
                    result.truncated = true;
                    return Ok(result);
                }
                if end > MAX_PREVIEW_BYTES {
                    continue;
                }
                result.hits.push(Hit {
                    path: rel.clone(),
                    line: line_no,
                    start,
                    end,
                    preview: preview(line),
                });
            }
        }
    }
    Ok(result)
}

fn preview(line: &str) -> String {
    if line.len() <= MAX_PREVIEW_BYTES {
        return line.to_string();
    }
    let mut cut = MAX_PREVIEW_BYTES;
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line[..cut].to_string()
}

/// The text of a file, or `None` when it is too big, unreadable, or binary (a NUL byte in its
/// first 8 KB). Invalid UTF-8 is replaced rather than refused.
fn read_text(path: &Path, max: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref().take(max + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max || bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn q(pattern: &str) -> SearchOptions {
        SearchOptions::new(Query {
            pattern: pattern.into(),
            ..Query::default()
        })
    }

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::create_dir_all(p.join("src")).expect("src");
        fs::create_dir_all(p.join("target")).expect("target");
        fs::write(p.join(".gitignore"), "target/\n").expect("ignore");
        fs::write(
            p.join("src/a.rs"),
            "fn alpha() {}\r\nfn beta() {}\nlet ALPHA = 1;\n",
        )
        .expect("a");
        fs::write(p.join("src/b.txt"), "nothing here\nalpha again\n").expect("b");
        fs::write(p.join("target/out.rs"), "alpha\n").expect("out");
        fs::write(p.join("blob.bin"), b"alpha\0\0binary").expect("blob");
        d
    }

    #[test]
    fn finds_matches_with_line_columns_and_skips_ignored_and_binary() {
        let d = project();
        let r = search(d.path(), &q("alpha"), &AtomicBool::new(false)).expect("search");
        let got: Vec<_> = r
            .hits
            .iter()
            .map(|h| {
                (
                    h.path.to_string_lossy().into_owned(),
                    h.line,
                    h.start,
                    h.end,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("src/a.rs".to_string(), 0, 3, 8),
                ("src/a.rs".to_string(), 2, 4, 9),
                ("src/b.txt".to_string(), 1, 0, 5),
            ]
        );
        assert_eq!(r.hits[0].preview, "fn alpha() {}");
        assert!(!r.truncated);
        assert_eq!(r.files_scanned, 3);
    }

    #[test]
    fn case_sensitive_regex_and_whole_word() {
        let d = project();
        let mut o = q("ALPHA");
        o.query.case_sensitive = true;
        let r = search(d.path(), &o, &AtomicBool::new(false)).expect("search");
        assert_eq!(r.hits.len(), 1);

        let mut o = q(r"f\w+ (a|b)");
        o.query.regex = true;
        let r = search(d.path(), &o, &AtomicBool::new(false)).expect("search");
        assert_eq!(r.hits.len(), 2);

        let mut o = q("alph");
        o.query.whole_word = true;
        assert!(search(d.path(), &o, &AtomicBool::new(false))
            .expect("search")
            .hits
            .is_empty());
    }

    #[test]
    fn a_bad_pattern_is_an_error_and_an_empty_one_is_too() {
        let d = project();
        let mut o = q("(");
        o.query.regex = true;
        assert!(search(d.path(), &o, &AtomicBool::new(false)).is_err());
        assert!(search(d.path(), &q(""), &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn limits_and_cancellation_say_they_cut_the_results() {
        let d = project();
        let mut o = q("alpha");
        o.max_results = 2;
        let r = search(d.path(), &o, &AtomicBool::new(false)).expect("search");
        assert_eq!((r.hits.len(), r.truncated), (2, true));
        let r = search(d.path(), &q("alpha"), &AtomicBool::new(true)).expect("search");
        assert!(r.hits.is_empty() && r.truncated);
    }

    #[test]
    fn big_files_and_very_long_lines_are_handled() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path();
        fs::write(p.join("big.txt"), "needle\n".repeat(10)).expect("big");
        let mut o = q("needle");
        o.max_file_bytes = 10;
        assert!(search(p, &o, &AtomicBool::new(false))
            .expect("search")
            .hits
            .is_empty());

        let long = format!(
            "{}ñ{}needle",
            "x".repeat(MAX_PREVIEW_BYTES - 1),
            "y".repeat(10)
        );
        fs::write(p.join("long.txt"), format!("{long}\nneedle\n")).expect("long");
        let r = search(p, &q("needle"), &AtomicBool::new(false)).expect("search");
        // The match past the preview cut is dropped; the one on the next line is kept.
        let long_hits: Vec<_> = r
            .hits
            .iter()
            .filter(|h| h.path.ends_with("long.txt"))
            .collect();
        assert_eq!(long_hits.len(), 1);
        assert_eq!(long_hits[0].line, 1);
        assert!(preview(&long).len() <= MAX_PREVIEW_BYTES);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_refused() {
        let d = tempfile::tempdir().expect("tempdir");
        fs::write(d.path().join("latin.txt"), b"caf\xe9 needle\n").expect("latin");
        let r = search(d.path(), &q("needle"), &AtomicBool::new(false)).expect("search");
        assert_eq!(r.hits.len(), 1);
    }
}
