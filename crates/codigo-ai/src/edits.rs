// SPDX-License-Identifier: MIT OR Apache-2.0
//! Changes the assistant proposes to files, and how to find where they go.
//!
//! The model is asked to answer in two plain shapes that work with any provider, no tool calls
//! needed. A change to an existing file is a fenced block whose first line says `edit` and the
//! path, holding one or more pairs of the old lines and the new ones:
//!
//! ````text
//! ```edit src/main.rs
//! <<<<<<< SEARCH
//! old lines
//! =======
//! new lines
//! >>>>>>> REPLACE
//! ```
//! ````
//!
//! A new file is a block that says `create` and the path, and holds the file. Nothing here touches
//! the disk: the app shows each proposal as a diff and applies the ones the person accepts.

use std::ops::Range;

/// What one proposal does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Put `replace` where `search` is.
    Replace { search: String, replace: String },
    /// Make a file that does not exist yet.
    Create { content: String },
}

/// One proposed change to one file, with the path as the model wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    pub path: String,
    pub change: Change,
}

/// The most proposals read from one answer, so a runaway reply cannot become a runaway list.
const MAX_EDITS: usize = 40;

fn fence_len(line: &str) -> usize {
    line.trim_start().chars().take_while(|c| *c == '`').count()
}

fn is_closing(line: &str, open: usize) -> bool {
    let t = line.trim();
    t.len() >= open && t.chars().all(|c| c == '`')
}

/// `edit src/a.rs`, `edit: src/a.rs` or `create "src/a.rs"` → the word and the path.
fn header(info: &str) -> Option<(&str, String)> {
    let info = info.trim();
    let split = info.find(|c: char| c.is_whitespace() || c == ':')?;
    let (word, rest) = info.split_at(split);
    let rest = rest
        .trim_start_matches(|c: char| c.is_whitespace() || c == ':')
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if rest.is_empty() {
        return None;
    }
    let word = ["edit", "create"]
        .into_iter()
        .find(|w| w.eq_ignore_ascii_case(word))?;
    Some((word, rest.to_owned()))
}

/// Every proposal in `reply`, in the order it was written. Anything that is not one of the two
/// shapes, and any half-written pair, is ignored.
pub fn parse(reply: &str) -> Vec<FileEdit> {
    let mut edits = Vec::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let open = fence_len(line);
        if open < 3 {
            continue;
        }
        let info = line.trim_start().trim_start_matches('`');
        let head = header(info);
        let mut body = Vec::new();
        for inner in lines.by_ref() {
            if is_closing(inner, open) {
                break;
            }
            body.push(inner);
        }
        let Some((word, path)) = head else {
            continue;
        };
        if word == "create" {
            let mut content = body.join("\n");
            content.push('\n');
            edits.push(FileEdit {
                path,
                change: Change::Create { content },
            });
        } else {
            pairs(&body, &path, &mut edits);
        }
        if edits.len() >= MAX_EDITS {
            edits.truncate(MAX_EDITS);
            break;
        }
    }
    edits
}

fn pairs(body: &[&str], path: &str, out: &mut Vec<FileEdit>) {
    #[derive(PartialEq)]
    enum Part {
        Outside,
        Search,
        Replace,
    }
    let (mut part, mut search, mut replace) = (Part::Outside, Vec::new(), Vec::new());
    for line in body {
        let t = line.trim_end();
        if t.starts_with("<<<<<<<") {
            part = Part::Search;
            search.clear();
            replace.clear();
        } else if part == Part::Search && t.starts_with("=======") {
            part = Part::Replace;
        } else if part == Part::Replace && t.starts_with(">>>>>>>") {
            out.push(FileEdit {
                path: path.to_owned(),
                change: Change::Replace {
                    search: search.join("\n"),
                    replace: replace.join("\n"),
                },
            });
            part = Part::Outside;
            search.clear();
            replace.clear();
        } else {
            match part {
                Part::Search => search.push(*line),
                Part::Replace => replace.push(*line),
                Part::Outside => {}
            }
        }
    }
}

/// `reply` without the `edit` and `create` blocks, which the app shows as diffs instead.
pub fn strip(reply: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let open = fence_len(line);
        let is_proposal = open >= 3 && header(line.trim_start().trim_start_matches('`')).is_some();
        if !is_proposal {
            out.push(line);
            continue;
        }
        for inner in lines.by_ref() {
            if is_closing(inner, open) {
                break;
            }
        }
    }
    let text = out.join("\n");
    // Dropping a block leaves blank lines behind.
    let mut cleaned = String::new();
    let mut blanks = 0;
    for l in text.lines() {
        if l.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        cleaned.push_str(l);
        cleaned.push('\n');
    }
    cleaned.trim().to_owned()
}

/// A path the model wrote, made safe to join to the project folder: relative, no `..`, no empty
/// parts, no NUL. `None` for anything that could leave the folder.
pub fn safe_relative(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_start_matches("./");
    if path.is_empty() || path.starts_with('/') || path.contains('\0') || path.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." | ".." => return None,
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// Why a `SEARCH` text could not be placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Locate {
    /// It is not in the file (the file changed, or the model misquoted it).
    Missing,
    /// It is in the file this many times, so there is no telling which one was meant.
    Ambiguous(usize),
    /// An empty search says nothing about where.
    Empty,
}

/// Where in `text` the old lines are, as a byte range. The text must be there exactly once;
/// when no exact copy exists, lines that differ only in trailing spaces count as the same.
pub fn locate(text: &str, search: &str) -> Result<Range<usize>, Locate> {
    if search.trim().is_empty() {
        return Err(Locate::Empty);
    }
    let mut found = text.match_indices(search);
    if let Some((start, _)) = found.next() {
        let more = found.count();
        return if more == 0 {
            Ok(start..start + search.len())
        } else {
            Err(Locate::Ambiguous(more + 1))
        };
    }
    locate_by_lines(text, search)
}

fn locate_by_lines(text: &str, search: &str) -> Result<Range<usize>, Locate> {
    let wanted: Vec<&str> = search
        .trim_matches('\n')
        .lines()
        .map(str::trim_end)
        .collect();
    if wanted.is_empty() {
        return Err(Locate::Empty);
    }
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = 0usize;
    for raw in text.split_inclusive('\n') {
        lines.push((at, raw.trim_end_matches(['\n', '\r'])));
        at += raw.len();
    }
    let mut hits = Vec::new();
    for i in 0..lines.len() {
        let Some(window) = lines.get(i..i + wanted.len()) else {
            break;
        };
        if window
            .iter()
            .zip(&wanted)
            .all(|((_, l), w)| l.trim_end() == *w)
        {
            let start = window.first().map_or(0, |(s, _)| *s);
            let end = window.last().map_or(start, |(s, l)| s + l.len());
            hits.push(start..end);
        }
    }
    match hits.len() {
        0 => Err(Locate::Missing),
        1 => Ok(hits.remove(0)),
        n => Err(Locate::Ambiguous(n)),
    }
}

/// The lines a proposal removes and adds, for showing it as a diff. An exact `search` is shown
/// as written; each is split into lines.
pub fn diff_lines(search: &str, replace: &str) -> (Vec<String>, Vec<String>) {
    let split = |s: &str| -> Vec<String> {
        if s.is_empty() {
            Vec::new()
        } else {
            s.lines().map(str::to_owned).collect()
        }
    };
    (split(search), split(replace))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = "I changed the greeting.\n\n```edit src/main.rs\n<<<<<<< SEARCH\nfn main() {\n    println!(\"hi\");\n}\n=======\nfn main() {\n    println!(\"hello\");\n}\n>>>>>>> REPLACE\n<<<<<<< SEARCH\nlet a = 1;\n=======\nlet a = 2;\n>>>>>>> REPLACE\n```\n\nAnd a new file:\n\n```create \"src/util.rs\"\npub fn one() -> u8 { 1 }\n```\n";

    #[test]
    fn edits_and_new_files_are_read_from_an_answer() {
        let edits = parse(REPLY);
        assert_eq!(edits.len(), 3);
        assert_eq!(edits[0].path, "src/main.rs");
        assert_eq!(
            edits[1].change,
            Change::Replace {
                search: "let a = 1;".into(),
                replace: "let a = 2;".into()
            }
        );
        assert_eq!(edits[2].path, "src/util.rs");
        assert_eq!(
            edits[2].change,
            Change::Create {
                content: "pub fn one() -> u8 { 1 }\n".into()
            }
        );
    }

    #[test]
    fn the_text_around_the_proposals_is_what_is_left_to_say() {
        let said = strip(REPLY);
        assert!(said.starts_with("I changed the greeting."));
        assert!(said.ends_with("And a new file:"));
        assert!(!said.contains("SEARCH") && !said.contains("pub fn one"));
        let plain = "Use this:\n\n```rust\nfn a() {}\n```";
        assert_eq!(strip(plain), plain);
    }

    #[test]
    fn plain_code_blocks_and_half_written_pairs_are_not_proposals() {
        assert!(parse("```rust\nfn a() {}\n```").is_empty());
        let cut = "```edit a.rs\n<<<<<<< SEARCH\nold\n=======\nnew\n";
        assert!(parse(cut).is_empty());
        assert!(parse("```edit\n<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE\n```").is_empty());
    }

    #[test]
    fn a_longer_fence_lets_the_code_hold_backticks() {
        let reply = "````edit README.md\n<<<<<<< SEARCH\n```\nold\n```\n=======\n```\nnew\n```\n>>>>>>> REPLACE\n````";
        let edits = parse(reply);
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].change,
            Change::Replace {
                search: "```\nold\n```".into(),
                replace: "```\nnew\n```".into()
            }
        );
    }

    #[test]
    fn a_runaway_answer_is_cut_to_a_bounded_list() {
        let one = "```create a.txt\nx\n```\n";
        assert_eq!(parse(&one.repeat(500)).len(), MAX_EDITS);
    }

    #[test]
    fn paths_that_could_leave_the_folder_are_refused() {
        assert_eq!(safe_relative("./src/a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(safe_relative("src\\a.rs").as_deref(), Some("src/a.rs"));
        for bad in [
            "",
            "/etc/passwd",
            "../a",
            "a/../../b",
            "C:/x",
            "a//b",
            "a\0b",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_old_text_must_be_in_the_file_exactly_once() {
        let text = "a\nb\nc\nb\n";
        assert_eq!(locate(text, "a\nb"), Ok(0..3));
        assert_eq!(locate(text, "b"), Err(Locate::Ambiguous(2)));
        assert_eq!(locate(text, "zzz"), Err(Locate::Missing));
        assert_eq!(locate(text, "  \n"), Err(Locate::Empty));
    }

    #[test]
    fn trailing_spaces_do_not_stop_a_match_and_the_line_end_is_kept() {
        let text = "fn a() {  \n    1\n}\r\nfn b() {}\n";
        let range = locate(text, "fn a() {\n    1\n}").unwrap_or(0..0);
        assert_eq!(&text[range], "fn a() {  \n    1\n}");
    }

    #[test]
    fn hostile_text_never_panics() {
        for s in [
            "",
            "\n",
            "é\n",
            "```",
            "<<<<<<<",
            ">>>>>>>\n=======\n<<<<<<<",
        ] {
            let _ = parse(s);
            let _ = locate(s, s);
            let _ = locate("é\né", s);
        }
    }
}
