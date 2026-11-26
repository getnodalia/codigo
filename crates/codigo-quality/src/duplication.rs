// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding code that was copied: runs of lines that appear again, in this file or another.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::rules::Finding;

/// Significant lines in a run before it counts as a copy.
const WINDOW: usize = 8;
/// Findings kept per project.
const MAX_FINDINGS: usize = 300;

/// A source file as the duplication check sees it.
pub(crate) struct Source<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

/// For each file, its copied blocks as findings (the later copy is the one reported).
pub(crate) fn find(sources: &[Source]) -> Vec<(usize, Finding)> {
    let mut seen: HashMap<u64, (usize, usize)> = HashMap::new();
    let mut out = Vec::new();
    for (file, source) in sources.iter().enumerate() {
        let lines = significant(source.text);
        let mut run: Option<(usize, usize, String)> = None;
        let flush = |run: &mut Option<(usize, usize, String)>, out: &mut Vec<(usize, Finding)>| {
            if let Some((first, last, other)) = run.take() {
                if let Some(f) = Finding::new("duplicated-block", first, 0, 0) {
                    let _ = last;
                    out.push((file, f.with_detail(other)));
                }
            }
        };
        for start in 0..lines.len().saturating_sub(WINDOW - 1) {
            let window = &lines[start..start + WINDOW];
            let mut hasher = DefaultHasher::new();
            for (_, text) in window {
                text.hash(&mut hasher);
            }
            let key = hasher.finish();
            let (first_line, last_line) = (window[0].0, window[WINDOW - 1].0);
            match seen.get(&key).copied() {
                Some((other_file, other_line))
                    if other_file != file || other_line + WINDOW <= first_line =>
                {
                    let place = format!(
                        "{}:{}",
                        sources.get(other_file).map_or("", |s| s.path),
                        other_line + 1
                    );
                    match run.as_mut() {
                        Some((_, last, _)) if first_line <= *last + 1 => *last = last_line,
                        _ => {
                            flush(&mut run, &mut out);
                            run = Some((first_line, last_line, place));
                        }
                    }
                }
                Some(_) => {}
                None => {
                    seen.insert(key, (file, first_line));
                }
            }
        }
        flush(&mut run, &mut out);
        if out.len() >= MAX_FINDINGS {
            out.truncate(MAX_FINDINGS);
            break;
        }
    }
    out
}

/// The lines that carry code: trimmed, with runs of blanks folded, without lone brackets, blank
/// lines and comments, each with its 0-based line number.
fn significant(text: &str) -> Vec<(usize, String)> {
    text.split('\n')
        .enumerate()
        .filter_map(|(n, raw)| {
            let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let trivial = line.len() < 6
                || line.starts_with("//")
                || line.starts_with('#')
                || line.starts_with("/*")
                || line.starts_with('*')
                || line.starts_with("use ")
                || line.starts_with("import ")
                || line.starts_with("from ");
            (!trivial).then_some((n, line))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> String {
        (0..10)
            .map(|i| format!("    let value_{i} = compute({i}) + other_{i};\n"))
            .collect()
    }

    #[test]
    fn a_copied_block_is_reported_once_at_the_later_copy() {
        let a = format!("fn a() {{\n{}}}\n", block());
        let b = format!("fn b() {{\n{}}}\n", block());
        let found = find(&[
            Source {
                path: "a.rs",
                text: &a,
            },
            Source {
                path: "b.rs",
                text: &b,
            },
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 1);
        assert_eq!(found[0].1.detail.as_deref(), Some("a.rs:2"));
    }

    #[test]
    fn different_code_and_short_lines_are_not_copies() {
        let a = "}\n".repeat(40);
        let b = "}\n".repeat(40);
        assert!(find(&[
            Source {
                path: "a",
                text: &a
            },
            Source {
                path: "b",
                text: &b
            }
        ])
        .is_empty());
        let c = (0..30)
            .map(|i| format!("let unique_line_number_{i} = {i} * {i};\n"))
            .collect::<String>();
        assert!(find(&[Source {
            path: "c",
            text: &c
        }])
        .is_empty());
    }
}
