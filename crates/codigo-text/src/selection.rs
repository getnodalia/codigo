// SPDX-License-Identifier: MIT OR Apache-2.0
//! Cursors and selections.

use std::ops::Range;

/// A selection: the text between `anchor` and `head`. The caret is at `head`; an empty selection
/// is a plain cursor. `goal` is the display column vertical moves aim for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    pub goal: Option<usize>,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Selection {
            anchor: at,
            head: at,
            goal: None,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Selection {
            anchor,
            head,
            goal: None,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// One or more selections, sorted and not overlapping, one of them primary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selections {
    items: Vec<Selection>,
    primary: usize,
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Selections {
            items: vec![selection],
            primary: 0,
        }
    }

    /// Sorted, merged where they overlap or touch a non-empty one; `primary` names the one that
    /// stays primary (an index into `items` as given). Empty input gives a cursor at 0.
    pub fn from_vec(items: Vec<Selection>, primary: usize) -> Self {
        let mut s = Selections { items, primary };
        s.normalize();
        s
    }

    pub fn as_slice(&self) -> &[Selection] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn primary(&self) -> Selection {
        self.items
            .get(self.primary)
            .or_else(|| self.items.first())
            .copied()
            .unwrap_or_else(|| Selection::caret(0))
    }

    pub fn primary_index(&self) -> usize {
        self.primary.min(self.items.len().saturating_sub(1))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.items.iter()
    }

    /// Keep only the primary selection.
    pub fn collapse_to_primary(&mut self) {
        let p = self.primary();
        self.items = vec![p];
        self.primary = 0;
    }

    pub fn push(&mut self, selection: Selection) {
        self.items.push(selection);
        self.primary = self.items.len() - 1;
        self.normalize();
    }

    pub fn map(&mut self, mut f: impl FnMut(Selection) -> Selection) {
        for s in &mut self.items {
            *s = f(*s);
        }
        self.normalize();
    }

    fn normalize(&mut self) {
        if self.items.is_empty() {
            self.items.push(Selection::caret(0));
            self.primary = 0;
            return;
        }
        let primary_item = self.items.get(self.primary).copied();
        let mut indexed: Vec<(usize, Selection)> = self.items.iter().copied().enumerate().collect();
        indexed.sort_by_key(|(i, s)| (s.start(), s.end(), *i));
        let mut merged: Vec<Selection> = Vec::with_capacity(indexed.len());
        let mut primary = 0usize;
        for (original, s) in indexed {
            let is_primary = self.primary == original;
            match merged.last_mut() {
                Some(last)
                    if s.start() < last.end()
                        || (s.start() == last.end() && (last.is_empty() || s.is_empty())) =>
                {
                    // Overlap, or a caret touching a selection: one selection covers both.
                    let start = last.start().min(s.start());
                    let end = last.end().max(s.end());
                    let forward = if is_primary {
                        s.head >= s.anchor
                    } else {
                        last.head >= last.anchor
                    };
                    *last = if forward {
                        Selection::new(start, end)
                    } else {
                        Selection::new(end, start)
                    };
                    if is_primary {
                        primary = merged.len() - 1;
                    }
                }
                _ => {
                    merged.push(s);
                    if is_primary {
                        primary = merged.len() - 1;
                    }
                }
            }
        }
        if primary_item.is_none() {
            primary = 0;
        }
        self.items = merged;
        self.primary = primary;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_sort_and_merge() {
        let s = Selections::from_vec(
            vec![
                Selection::new(10, 14),
                Selection::new(2, 4),
                Selection::new(12, 16),
            ],
            0,
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s.as_slice()[0].range(), 2..4);
        assert_eq!(s.as_slice()[1].range(), 10..16);
        assert_eq!(s.primary().range(), 10..16);
    }

    #[test]
    fn identical_carets_become_one() {
        let s = Selections::from_vec(vec![Selection::caret(3), Selection::caret(3)], 1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn a_backwards_selection_stays_backwards() {
        let s = Selections::from_vec(vec![Selection::new(8, 2), Selection::new(5, 6)], 0);
        assert_eq!(s.len(), 1);
        assert_eq!((s.primary().anchor, s.primary().head), (8, 2));
    }

    #[test]
    fn empty_input_gives_a_cursor() {
        let s = Selections::from_vec(Vec::new(), 0);
        assert_eq!(s.primary(), Selection::caret(0));
    }
}
