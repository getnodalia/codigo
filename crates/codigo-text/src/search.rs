// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding and replacing text, line by line.

use regex::{Regex, RegexBuilder};
use ropey::Rope;

use crate::edit::{line_end_char, Edit};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

/// A match, as character offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

/// A compiled [`Query`]. Matches never span lines.
pub struct Matcher {
    re: Regex,
}

/// A pattern that is too big or too slow is refused rather than allowed to hang the editor.
const SIZE_LIMIT: usize = 1 << 20;

impl Matcher {
    pub fn new(query: &Query) -> Result<Matcher, String> {
        if query.pattern.is_empty() {
            return Err(String::new());
        }
        let mut pattern = if query.regex {
            query.pattern.clone()
        } else {
            regex::escape(&query.pattern)
        };
        if query.whole_word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case_sensitive)
            .multi_line(true)
            .size_limit(SIZE_LIMIT)
            .dfa_size_limit(SIZE_LIMIT * 8)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Matcher { re })
    }

    /// Matches in one line of text, as byte ranges. Empty matches are left out.
    pub fn in_text(&self, text: &str) -> Vec<(usize, usize)> {
        self.re
            .find_iter(text)
            .filter(|m| m.start() != m.end())
            .map(|m| (m.start(), m.end()))
            .collect()
    }

    /// Up to `limit` matches in the whole rope.
    pub fn find_all(&self, rope: &Rope, limit: usize) -> Vec<Match> {
        let mut out = Vec::new();
        for line in 0..rope.len_lines() {
            let start = rope.line_to_char(line);
            let end = line_end_char(rope, line);
            if end == start {
                continue;
            }
            let text = rope.slice(start..end).to_string();
            let ascii = text.is_ascii();
            for (b0, b1) in self.in_text(&text) {
                let (c0, c1) = if ascii {
                    (b0, b1)
                } else {
                    (text[..b0].chars().count(), text[..b1].chars().count())
                };
                out.push(Match {
                    start: start + c0,
                    end: start + c1,
                });
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }

    /// The text that replaces `found`, with `$1`-style groups expanded when the query is a regex.
    pub fn replacement(&self, rope: &Rope, found: Match, replacement: &str, regex: bool) -> String {
        if !regex {
            return replacement.to_owned();
        }
        let line = rope.char_to_line(found.start);
        let line_start = rope.line_to_char(line);
        let text = rope
            .slice(line_start..line_end_char(rope, line))
            .to_string();
        let byte_start = text
            .char_indices()
            .nth(found.start - line_start)
            .map_or(text.len(), |(b, _)| b);
        match self.re.captures_at(&text, byte_start) {
            Some(caps) if caps.get(0).is_some_and(|m| m.start() == byte_start) => {
                let mut out = String::new();
                caps.expand(replacement, &mut out);
                out
            }
            _ => replacement.to_owned(),
        }
    }

    /// The edits that replace each match.
    pub fn replace_edits(
        &self,
        rope: &Rope,
        matches: &[Match],
        replacement: &str,
        regex: bool,
    ) -> Vec<Edit> {
        matches
            .iter()
            .map(|m| {
                Edit::new(
                    m.start..m.end,
                    self.replacement(rope, *m, replacement, regex),
                )
            })
            .collect()
    }
}

/// Up to `limit` matches of `query` in `rope`.
pub fn find_all(rope: &Rope, query: &Query, limit: usize) -> Result<Vec<Match>, String> {
    Ok(Matcher::new(query)?.find_all(rope, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pattern: &str) -> Query {
        Query {
            pattern: pattern.into(),
            ..Query::default()
        }
    }

    #[test]
    fn literal_search_ignores_case_by_default() {
        let rope = Rope::from_str("Foo foo FOO\nbar foo");
        let found = find_all(&rope, &q("foo"), 100).unwrap();
        assert_eq!(found.len(), 4);
        assert_eq!(found[3], Match { start: 16, end: 19 });
        let exact = Query {
            case_sensitive: true,
            ..q("foo")
        };
        assert_eq!(find_all(&rope, &exact, 100).unwrap().len(), 2);
    }

    #[test]
    fn whole_word_and_regex() {
        let rope = Rope::from_str("cat concat cat_x cat");
        let word = Query {
            whole_word: true,
            ..q("cat")
        };
        assert_eq!(find_all(&rope, &word, 100).unwrap().len(), 2);
        let re = Query {
            regex: true,
            ..q(r"c\w+t")
        };
        assert_eq!(find_all(&rope, &re, 100).unwrap().len(), 4);
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let rope = Rope::from_str("é🙂 x\nñ x");
        let found = find_all(&rope, &q("x"), 10).unwrap();
        assert_eq!(found[0], Match { start: 3, end: 4 });
        assert_eq!(found[1], Match { start: 7, end: 8 });
    }

    #[test]
    fn a_bad_pattern_is_an_error_and_an_empty_one_finds_nothing() {
        let bad = Query {
            regex: true,
            ..q("(")
        };
        assert!(find_all(&Rope::from_str("x"), &bad, 10).is_err());
        assert_eq!(
            find_all(&Rope::from_str("x"), &q(""), 10),
            Err(String::new())
        );
    }

    #[test]
    fn the_limit_stops_the_search() {
        let rope = Rope::from_str("a a a a a");
        assert_eq!(find_all(&rope, &q("a"), 3).unwrap().len(), 3);
    }

    #[test]
    fn regex_replacements_expand_groups() {
        let rope = Rope::from_str("key=1\nkey=22");
        let query = Query {
            regex: true,
            ..q(r"key=(\d+)")
        };
        let m = Matcher::new(&query).unwrap();
        let found = m.find_all(&rope, 10);
        let edits = m.replace_edits(&rope, &found, "v$1", true);
        assert_eq!(edits[0].text, "v1");
        assert_eq!(edits[1].text, "v22");
        let literal = m.replace_edits(&rope, &found, "v$1", false);
        assert_eq!(literal[0].text, "v$1");
    }
}
