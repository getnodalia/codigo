// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the caret goes: graphemes, words, display columns.

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

use crate::edit::line_end_char;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharClass {
    Word,
    Space,
    Punct,
    Newline,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn class(c: char) -> CharClass {
    if c == '\n' {
        CharClass::Newline
    } else if c.is_whitespace() {
        CharClass::Space
    } else if is_word_char(c) {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Lines longer than this move one character at a time instead of one grapheme.
const GRAPHEME_LINE_LIMIT: usize = 4096;

/// The offset one grapheme cluster after `idx`.
pub fn next_grapheme(rope: &Rope, idx: usize) -> usize {
    let len = rope.len_chars();
    if idx >= len {
        return len;
    }
    let line = rope.char_to_line(idx);
    let start = rope.line_to_char(line);
    let end = (line_end_char(rope, line) + 1).min(len);
    if end - start > GRAPHEME_LINE_LIMIT {
        return idx + 1;
    }
    let text = rope.slice(start..end).to_string();
    let mut offset = 0usize;
    for g in text.graphemes(true) {
        let next = offset + g.chars().count();
        if start + next > idx {
            return start + next;
        }
        offset = next;
    }
    (idx + 1).min(len)
}

/// The offset one grapheme cluster before `idx`.
pub fn prev_grapheme(rope: &Rope, idx: usize) -> usize {
    if idx == 0 {
        return 0;
    }
    let idx = idx.min(rope.len_chars());
    let line = rope.char_to_line(idx - 1);
    let start = rope.line_to_char(line);
    let end = (line_end_char(rope, line) + 1).min(rope.len_chars());
    if end - start > GRAPHEME_LINE_LIMIT {
        return idx - 1;
    }
    let text = rope.slice(start..end).to_string();
    let mut offset = 0usize;
    for g in text.graphemes(true) {
        let next = offset + g.chars().count();
        if start + next >= idx {
            return start + offset;
        }
        offset = next;
    }
    idx - 1
}

/// The start of the word before `idx` (VS Code's Ctrl+Left): skip blanks, then one run of the
/// same kind of character. At a line start, step to the end of the line before.
pub fn word_start_before(rope: &Rope, idx: usize) -> usize {
    let mut i = idx.min(rope.len_chars());
    if i == 0 {
        return 0;
    }
    if rope.char(i - 1) == '\n' {
        return i - 1;
    }
    while i > 0 && class(rope.char(i - 1)) == CharClass::Space {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    let k = class(rope.char(i - 1));
    if k == CharClass::Newline {
        return i;
    }
    while i > 0 && class(rope.char(i - 1)) == k {
        i -= 1;
    }
    i
}

/// The end of the word after `idx` (Ctrl+Right).
pub fn word_end_after(rope: &Rope, idx: usize) -> usize {
    let len = rope.len_chars();
    let mut i = idx.min(len);
    if i >= len {
        return len;
    }
    if rope.char(i) == '\n' {
        return i + 1;
    }
    while i < len && class(rope.char(i)) == CharClass::Space {
        i += 1;
    }
    if i >= len {
        return len;
    }
    let k = class(rope.char(i));
    if k == CharClass::Newline {
        return i;
    }
    while i < len && class(rope.char(i)) == k {
        i += 1;
    }
    i
}

/// The word (or run of punctuation) around `idx`, for double click.
pub fn word_range_at(rope: &Rope, idx: usize) -> (usize, usize) {
    let len = rope.len_chars();
    let idx = idx.min(len);
    let here = if idx < len {
        Some(class(rope.char(idx)))
    } else {
        None
    };
    let before = if idx > 0 {
        Some(class(rope.char(idx - 1)))
    } else {
        None
    };
    let k = match (here, before) {
        (Some(CharClass::Word), _) => CharClass::Word,
        (_, Some(CharClass::Word)) => CharClass::Word,
        (Some(c), _) if c != CharClass::Newline => c,
        (_, Some(c)) if c != CharClass::Newline => c,
        _ => return (idx, idx),
    };
    let mut start = idx;
    while start > 0 && class(rope.char(start - 1)) == k {
        start -= 1;
    }
    let mut end = idx;
    while end < len && class(rope.char(end)) == k {
        end += 1;
    }
    (start, end)
}

/// The width in columns of `c` at display column `col`.
pub fn char_width(c: char, col: usize, tab_width: usize) -> usize {
    if c == '\t' {
        let tab = tab_width.max(1);
        tab - col % tab
    } else {
        c.width().unwrap_or(0)
    }
}

/// The display column of the character offset `idx`.
pub fn display_col(rope: &Rope, idx: usize, tab_width: usize) -> usize {
    let idx = idx.min(rope.len_chars());
    let start = rope.line_to_char(rope.char_to_line(idx));
    let mut col = 0;
    for c in rope.slice(start..idx).chars() {
        col += char_width(c, col, tab_width);
    }
    col
}

/// The character offset on `line` closest to display column `col` (not past the line's end).
pub fn offset_at_display_col(rope: &Rope, line: usize, col: usize, tab_width: usize) -> usize {
    let start = rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)));
    let end = line_end_char(rope, line);
    let mut at = 0;
    let mut idx = start;
    for c in rope.slice(start..end).chars() {
        let w = char_width(c, at, tab_width);
        if at + w > col && at + w - col > col.saturating_sub(at) {
            break;
        }
        if at >= col && w > 0 {
            break;
        }
        at += w;
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_move_whole_clusters() {
        let rope = Rope::from_str("a👨‍👩‍👧b\ne\u{301}x");
        let family_end = next_grapheme(&rope, 1);
        assert_eq!(rope.slice(1..family_end).to_string(), "👨‍👩‍👧");
        assert_eq!(prev_grapheme(&rope, family_end), 1);
        let line2 = rope.line_to_char(1);
        assert_eq!(
            next_grapheme(&rope, line2),
            line2 + 2,
            "e + combining accent"
        );
        assert_eq!(prev_grapheme(&rope, line2 + 2), line2);
    }

    #[test]
    fn graphemes_stop_at_the_ends() {
        let rope = Rope::from_str("ab");
        assert_eq!(next_grapheme(&rope, 2), 2);
        assert_eq!(prev_grapheme(&rope, 0), 0);
        assert_eq!(next_grapheme(&rope, 0), 1);
        assert_eq!(prev_grapheme(&rope, 2), 1);
    }

    #[test]
    fn words_skip_blanks_then_one_run() {
        let rope = Rope::from_str("foo_bar  (baz)\nnext");
        assert_eq!(word_end_after(&rope, 0), 7);
        assert_eq!(word_end_after(&rope, 7), 10, "blanks, then the paren");
        assert_eq!(word_start_before(&rope, 7), 0);
        assert_eq!(word_start_before(&rope, 12), 10);
        assert_eq!(word_end_after(&rope, 14), 15, "over the newline");
        assert_eq!(word_start_before(&rope, 15), 14);
    }

    #[test]
    fn double_click_picks_the_word() {
        let rope = Rope::from_str("let foo_1 = 2;");
        assert_eq!(word_range_at(&rope, 5), (4, 9));
        assert_eq!(word_range_at(&rope, 9), (4, 9), "at the end of the word");
        assert_eq!(word_range_at(&rope, 10), (10, 11));
    }

    #[test]
    fn tabs_and_wide_characters_count_columns() {
        let rope = Rope::from_str("\tx漢y");
        assert_eq!(display_col(&rope, 1, 4), 4);
        assert_eq!(display_col(&rope, 3, 4), 7);
        assert_eq!(offset_at_display_col(&rope, 0, 5, 4), 2);
        assert_eq!(offset_at_display_col(&rope, 0, 99, 4), 4);
        assert_eq!(offset_at_display_col(&rope, 0, 0, 4), 0);
    }
}
