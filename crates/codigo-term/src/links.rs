// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding `path:line:column` references in terminal output, the way compilers, linters and test
//! runners print them, so a click can jump to the place they name.

/// A reference to a place in a file: `line` and `column` are 1-based as printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path: String,
    pub line: usize,
    pub column: Option<usize>,
}

/// The reference under character `at` of `row`, if the word there is one.
pub fn file_ref_at(row: &[char], at: usize) -> Option<FileRef> {
    let is_edge = |c: char| c.is_whitespace() || "()[]<>\"'`,;|{}".contains(c);
    if at >= row.len() || is_edge(*row.get(at)?) {
        return None;
    }
    let mut start = at;
    while start > 0 && row.get(start - 1).is_some_and(|c| !is_edge(*c)) {
        start -= 1;
    }
    let mut end = at + 1;
    while row.get(end).is_some_and(|c| !is_edge(*c)) {
        end += 1;
    }
    let word: String = row.get(start..end)?.iter().collect();
    parse(word.trim_end_matches(['.', ':', '!', '?']))
}

/// Parse `path:line` or `path:line:column`. The path must look like a file (a separator or an
/// extension), so times like `12:30` and ports like `localhost:8080` are not references.
pub fn parse(word: &str) -> Option<FileRef> {
    let mut parts = word.rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    let (path, line, column) = match (middle.parse::<usize>(), parts.next()) {
        (Ok(line), Some(path)) => (path, line, last.parse::<usize>().ok()),
        _ => (
            word.get(..word.len().checked_sub(last.len() + 1)?)?,
            last.parse().ok()?,
            None,
        ),
    };
    if line == 0 || path.is_empty() || column == Some(0) {
        return None;
    }
    let path = path.strip_prefix("file://").unwrap_or(path);
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let looks_like_file = path.contains(['/', '\\']) || name.contains('.');
    let host_port = !path.contains(['/', '\\']) && name.chars().all(|c| c.is_ascii_alphabetic());
    (looks_like_file && !host_port && !name.is_empty()).then(|| FileRef {
        path: path.to_owned(),
        line,
        column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, col: usize) -> Option<FileRef> {
        file_ref_at(&text.chars().collect::<Vec<_>>(), col)
    }

    #[test]
    fn compiler_output_is_recognised() {
        let r = at(" --> src/main.rs:2:18", 8).unwrap();
        assert_eq!(
            (r.path.as_str(), r.line, r.column),
            ("src/main.rs", 2, Some(18))
        );
        let r = at("src/lib.rs:40: warning", 3).unwrap();
        assert_eq!(
            (r.path.as_str(), r.line, r.column),
            ("src/lib.rs", 40, None)
        );
    }

    #[test]
    fn wrapped_and_trailing_punctuation_is_ignored() {
        let r = at("(app/main.py:7:3).", 4).unwrap();
        assert_eq!(r.path, "app/main.py");
        let r = at("error at ./a.ts:10:5:", 12).unwrap();
        assert_eq!((r.path.as_str(), r.line, r.column), ("./a.ts", 10, Some(5)));
    }

    #[test]
    fn times_ports_and_plain_words_are_not_references() {
        assert_eq!(at("at 12:30 today", 4), None);
        assert_eq!(at("listening on localhost:8080", 22), None);
        assert_eq!(at("note: something", 1), None);
        assert_eq!(at("src/main.rs:0", 2), None);
        assert_eq!(at("src/main.rs:12", 99), None);
        assert_eq!(at("a b", 1), None);
    }

    #[test]
    fn windows_paths_and_unicode_do_not_panic() {
        let r = parse("src\\main.rs:3").unwrap();
        assert_eq!(r.path, "src\\main.rs");
        assert_eq!(parse("ñandú/é.rs:9:9").map(|r| r.line), Some(9));
        assert_eq!(parse(":::"), None);
        assert_eq!(parse(""), None);
    }
}
