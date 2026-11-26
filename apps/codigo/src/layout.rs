// SPDX-License-Identifier: MIT OR Apache-2.0
//! Text as a grid of cells: how wide each character is on screen, and how long lines wrap.
//!
//! The editor draws on a fixed grid so that a position on screen and a character offset can be
//! converted into each other with arithmetic, not by asking a font. A tab is as many cells as it
//! takes to reach the next tab stop, a wide character (CJK, most emoji) takes two, a combining
//! mark none.

use unicode_width::UnicodeWidthChar;

/// Cells a character takes when it starts at cell `col`.
pub fn char_cells(c: char, col: usize, tab: usize) -> usize {
    match c {
        '\t' => {
            let tab = tab.max(1);
            tab - col % tab
        }
        '\n' | '\r' => 0,
        c => c.width().unwrap_or(1),
    }
}

/// The cell each character of `line` starts at, plus one more for its end: `len == chars + 1`.
pub fn cells_of(line: &str, tab: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(line.len() + 1);
    let mut col = 0usize;
    for c in line.chars() {
        out.push(col as u32);
        col = col.saturating_add(char_cells(c, col, tab));
    }
    out.push(col as u32);
    out
}

/// Width of a line in cells, without building the table when the line is plain ASCII.
pub fn width_of(line: &str, tab: usize) -> usize {
    if line.is_ascii() && !line.contains('\t') {
        return line.len();
    }
    let mut col = 0usize;
    for c in line.chars() {
        col = col.saturating_add(char_cells(c, col, tab));
    }
    col
}

/// The character ranges a line breaks into when at most `max_cols` cells fit on a row. Rows
/// break after a space when one is close, otherwise in the middle of a word. Always at least one
/// row, and every row at least one character.
pub fn wrap(line: &str, cells: &[u32], max_cols: usize) -> Vec<(usize, usize)> {
    let chars = cells.len().saturating_sub(1);
    let max_cols = max_cols.max(2) as u32;
    let mut rows = Vec::new();
    let mut start = 0usize;
    if chars == 0 {
        return vec![(0, 0)];
    }
    let letters: Vec<char> = line.chars().collect();
    while start < chars {
        let base = cells.get(start).copied().unwrap_or(0);
        // The farthest end such that the row fits.
        let mut end = start + 1;
        while end < chars && cells.get(end + 1).copied().unwrap_or(u32::MAX) - base <= max_cols {
            end += 1;
        }
        if end < chars {
            // Prefer to break after the last space inside the row, if it is not too far back.
            let floor = start + (end - start) / 2;
            if let Some(space) = (floor..end)
                .rev()
                .find(|i| letters.get(*i).is_some_and(|c| *c == ' ' || *c == '\t'))
            {
                end = space + 1;
            }
        }
        rows.push((start, end));
        start = end;
    }
    rows
}

/// The index of the character nearest to cell `col` among `cells[from..=to]`.
pub fn char_at_col(cells: &[u32], from: usize, to: usize, col: f32) -> usize {
    let to = to.min(cells.len().saturating_sub(1));
    let from = from.min(to);
    let base = cells.get(from).copied().unwrap_or(0) as f32;
    let target = col + base;
    // First index whose start is at or past the target.
    let slice = cells.get(from..=to).unwrap_or(&[]);
    let at = slice.partition_point(|c| (*c as f32) < target);
    let i = from + at;
    if i == from {
        return from;
    }
    if i > to {
        return to;
    }
    let before = cells.get(i - 1).copied().unwrap_or(0) as f32;
    let here = cells.get(i).copied().unwrap_or(0) as f32;
    // Nearer to the character on the left or the right of the boundary.
    if target - before < here - target {
        i - 1
    } else {
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_stop_at_multiples_wide_characters_take_two() {
        assert_eq!(cells_of("a\tb", 4), vec![0, 1, 4, 5]);
        assert_eq!(cells_of("\t\t", 4), vec![0, 4, 8]);
        assert_eq!(cells_of("日本", 4), vec![0, 2, 4]);
        assert_eq!(cells_of("é", 4).len(), 2);
        assert_eq!(width_of("abc", 4), 3);
        assert_eq!(width_of("a\tb", 4), 5);
        assert_eq!(width_of("", 4), 0);
        assert_eq!(char_cells('\t', 3, 4), 1);
    }

    #[test]
    fn wrapping_breaks_after_spaces_and_never_loops() {
        let line = "hello world again";
        let cells = cells_of(line, 4);
        let rows = wrap(line, &cells, 8);
        assert_eq!(rows, vec![(0, 6), (6, 12), (12, 17)]);
        let long = "x".repeat(25);
        let rows = wrap(&long, &cells_of(&long, 4), 10);
        assert_eq!(rows, vec![(0, 10), (10, 20), (20, 25)]);
        assert_eq!(wrap("", &cells_of("", 4), 10), vec![(0, 0)]);
        // A width of zero or one still makes progress.
        let rows = wrap("abc", &cells_of("abc", 4), 0);
        assert_eq!(rows.last().map(|r| r.1), Some(3));
        let rows = wrap("日本語", &cells_of("日本語", 4), 3);
        assert!(rows.iter().all(|(a, b)| b > a));
    }

    #[test]
    fn a_column_maps_to_the_nearest_character_boundary() {
        let cells = cells_of("abcd", 4);
        assert_eq!(char_at_col(&cells, 0, 4, 0.2), 0);
        assert_eq!(char_at_col(&cells, 0, 4, 0.6), 1);
        assert_eq!(char_at_col(&cells, 0, 4, 2.4), 2);
        assert_eq!(char_at_col(&cells, 0, 4, 99.0), 4);
        assert_eq!(char_at_col(&cells, 0, 4, -3.0), 0);
        let wide = cells_of("日本", 4);
        assert_eq!(char_at_col(&wide, 0, 2, 0.9), 0);
        assert_eq!(char_at_col(&wide, 0, 2, 1.1), 1);
        // On a wrapped row that starts at character 2.
        let cells = cells_of("abcdef", 4);
        assert_eq!(char_at_col(&cells, 2, 4, 1.4), 3);
        assert_eq!(char_at_col(&cells, 2, 4, 50.0), 4);
    }
}
