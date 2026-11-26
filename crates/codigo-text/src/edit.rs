// SPDX-License-Identifier: MIT OR Apache-2.0
//! Edits and the changes they leave behind.

use std::ops::Range;

use ropey::Rope;

/// Replace the characters in `range` with `text`. Offsets are character offsets into the rope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

impl Edit {
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Edit {
            range,
            text: text.into(),
        }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Edit::new(at..at, text)
    }

    pub fn delete(range: Range<usize>) -> Self {
        Edit::new(range, String::new())
    }
}

/// What one applied edit did to the text, in the coordinates syntax trees (bytes and points) and
/// language servers (UTF-16 lines and columns) use. Changes come in the order they were applied,
/// each relative to the text left by the one before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start_char: usize,
    pub old_end_char: usize,
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    /// `(row, column in bytes)`.
    pub start_point: (usize, usize),
    pub old_end_point: (usize, usize),
    pub new_end_point: (usize, usize),
    /// `(line, column in UTF-16 units)`.
    pub lsp_start: (u32, u32),
    pub lsp_end: (u32, u32),
    /// The text that took the place of the removed range.
    pub text: String,
}

/// `(row, column in bytes)` of a character offset.
pub fn point_of(rope: &Rope, char_idx: usize) -> (usize, usize) {
    let char_idx = char_idx.min(rope.len_chars());
    let row = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(row);
    let column = rope.char_to_byte(char_idx) - rope.char_to_byte(line_start);
    (row, column)
}

/// `(line, column in UTF-16 units)` of a character offset.
pub fn lsp_position(rope: &Rope, char_idx: usize) -> (u32, u32) {
    let char_idx = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(line);
    let units: usize = rope
        .slice(line_start..char_idx)
        .chars()
        .map(char::len_utf16)
        .sum();
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(units).unwrap_or(u32::MAX),
    )
}

/// The character offset of an LSP position, clamped to the line it names.
pub fn char_of_lsp(rope: &Rope, line: u32, utf16: u32) -> usize {
    let line = usize::try_from(line).unwrap_or(usize::MAX);
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let start = rope.line_to_char(line);
    let end = line_end_char(rope, line);
    let mut units = 0u32;
    let mut idx = start;
    for c in rope.slice(start..end).chars() {
        if units >= utf16 {
            break;
        }
        units = units.saturating_add(u32::try_from(c.len_utf16()).unwrap_or(1));
        idx += 1;
    }
    idx
}

/// The offset of the end of `line`, before its `\n`.
pub fn line_end_char(rope: &Rope, line: usize) -> usize {
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let start = rope.line_to_char(line);
    let next = if line + 1 < rope.len_lines() {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars()
    };
    let mut end = next;
    if end > start && rope.char(end - 1) == '\n' {
        end -= 1;
    }
    end
}

fn point_after(start: (usize, usize), text: &str) -> (usize, usize) {
    let newlines = text.matches('\n').count();
    if newlines == 0 {
        (start.0, start.1 + text.len())
    } else {
        let last = text.rfind('\n').map_or(0, |i| i + 1);
        (start.0 + newlines, text.len() - last)
    }
}

/// Apply `edits` to `rope`. They must be sorted by start and must not overlap; `None` means they
/// did not satisfy that (nothing is changed). On success: the edits that undo these (in the
/// coordinates of the new text) and the changes, in the order they were applied.
pub fn apply(rope: &mut Rope, edits: &[Edit]) -> Option<(Vec<Edit>, Vec<Change>)> {
    let len = rope.len_chars();
    let mut previous_end = 0usize;
    for edit in edits {
        if edit.range.start > edit.range.end
            || edit.range.end > len
            || edit.range.start < previous_end
        {
            return None;
        }
        previous_end = edit.range.end;
    }
    // The undo edits, in the coordinates of the text after all the edits.
    let mut inverse = Vec::with_capacity(edits.len());
    let mut delta: isize = 0;
    for edit in edits {
        let removed: String = rope.slice(edit.range.clone()).to_string();
        let start = edit.range.start as isize + delta;
        let new_len = edit.text.chars().count();
        let start = usize::try_from(start).unwrap_or(0);
        inverse.push(Edit::new(start..start + new_len, removed));
        delta += new_len as isize - (edit.range.end - edit.range.start) as isize;
    }
    // Apply back to front, so the offsets of the earlier edits stay valid.
    let mut changes = Vec::with_capacity(edits.len());
    for edit in edits.iter().rev() {
        let start_point = point_of(rope, edit.range.start);
        let old_end_point = point_of(rope, edit.range.end);
        let lsp_start = lsp_position(rope, edit.range.start);
        let lsp_end = lsp_position(rope, edit.range.end);
        let start_byte = rope.char_to_byte(edit.range.start);
        let old_end_byte = rope.char_to_byte(edit.range.end);
        if edit.range.end > edit.range.start {
            rope.remove(edit.range.clone());
        }
        if !edit.text.is_empty() {
            rope.insert(edit.range.start, &edit.text);
        }
        changes.push(Change {
            start_char: edit.range.start,
            old_end_char: edit.range.end,
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + edit.text.len(),
            start_point,
            old_end_point,
            new_end_point: point_after(start_point, &edit.text),
            lsp_start,
            lsp_end,
            text: edit.text.clone(),
        });
    }
    Some((inverse, changes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_apply_back_to_front_and_invert() {
        let mut rope = Rope::from_str("hello world\nsecond");
        let edits = vec![Edit::new(0..5, "HEY"), Edit::new(6..11, "there")];
        let (inverse, changes) = apply(&mut rope, &edits).unwrap();
        assert_eq!(rope.to_string(), "HEY there\nsecond");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].start_char, 6, "the last edit goes first");
        let (_, _) = apply(&mut rope, &inverse).unwrap();
        assert_eq!(rope.to_string(), "hello world\nsecond");
    }

    #[test]
    fn overlapping_or_out_of_range_edits_change_nothing() {
        let mut rope = Rope::from_str("abc");
        assert!(apply(&mut rope, &[Edit::new(0..2, "x"), Edit::new(1..3, "y")]).is_none());
        assert!(apply(&mut rope, &[Edit::new(0..9, "x")]).is_none());
        #[allow(clippy::reversed_empty_ranges)]
        let backwards = Edit::new(2..1, "x");
        assert!(apply(&mut rope, &[backwards]).is_none());
        assert_eq!(rope.to_string(), "abc");
    }

    #[test]
    fn changes_carry_bytes_points_and_utf16() {
        let mut rope = Rope::from_str("aé😀\nxyz");
        // Replace "😀\nx" (chars 2..5) with "Q\nR\n".
        let (_, changes) = apply(&mut rope, &[Edit::new(2..5, "Q\nR\n")]).unwrap();
        let c = &changes[0];
        assert_eq!(c.start_byte, 3);
        assert_eq!(c.old_end_byte, 3 + 4 + 1 + 1);
        assert_eq!(c.new_end_byte, 3 + 4);
        assert_eq!(c.start_point, (0, 3));
        assert_eq!(c.old_end_point, (1, 1));
        assert_eq!(c.new_end_point, (2, 0));
        assert_eq!(c.lsp_start, (0, 2));
        assert_eq!(c.lsp_end, (1, 1));
        assert_eq!(rope.to_string(), "aéQ\nR\nyz");
    }

    #[test]
    fn lsp_positions_round_trip() {
        let rope = Rope::from_str("a😀b\nsecond");
        assert_eq!(lsp_position(&rope, 2), (0, 3));
        assert_eq!(char_of_lsp(&rope, 0, 3), 2);
        assert_eq!(char_of_lsp(&rope, 0, 99), 3, "clamped to the line");
        assert_eq!(char_of_lsp(&rope, 1, 2), 6);
        assert_eq!(char_of_lsp(&rope, 9, 0), rope.len_chars());
    }

    #[test]
    fn line_end_leaves_out_the_newline() {
        let rope = Rope::from_str("ab\n\ncd");
        assert_eq!(line_end_char(&rope, 0), 2);
        assert_eq!(line_end_char(&rope, 1), 3);
        assert_eq!(line_end_char(&rope, 2), 6);
    }
}
