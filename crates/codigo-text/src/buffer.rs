// SPDX-License-Identifier: MIT OR Apache-2.0
//! The buffer: text, cursors, history and the editing commands.

use std::ops::Range;
use std::time::Instant;

use ropey::Rope;

use crate::edit::{self, line_end_char, Change, Edit};
use crate::encoding::{Decoded, LineEnding};
use crate::history::{History, Step};
use crate::motion;
use crate::selection::{Selection, Selections};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
}

impl Indent {
    pub fn unit(self) -> String {
        match self {
            Indent::Tabs => "\t".to_owned(),
            Indent::Spaces(n) => " ".repeat(n.clamp(1, 16)),
        }
    }
}

/// Characters that are typed in pairs.
#[derive(Clone, Copy, Debug)]
pub struct Pairs(pub &'static [(char, char)]);

impl Pairs {
    pub const NONE: Pairs = Pairs(&[]);
    pub const DEFAULT: Pairs = Pairs(&[
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
    ]);

    fn closing(&self, open: char) -> Option<char> {
        self.0.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
    }

    fn is_closing(&self, c: char) -> bool {
        self.0.iter().any(|(_, close)| *close == c)
    }
}

/// When a new line is indented one level deeper than the one above.
#[derive(Clone, Copy, Debug)]
pub struct IndentRules {
    /// A line ending in one of these opens a block.
    pub open: &'static str,
    /// A line ending in `:` opens a block (Python, YAML).
    pub colon: bool,
}

impl Default for IndentRules {
    fn default() -> Self {
        IndentRules {
            open: "{[(",
            colon: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    /// First non-blank of the line; pressed again, the very start.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp(usize),
    PageDown(usize),
    ParagraphUp,
    ParagraphDown,
}

#[derive(Clone, Debug)]
pub struct Buffer {
    rope: Rope,
    selections: Selections,
    history: History,
    revision: u64,
    line_ending: LineEnding,
    bom: bool,
    pub indent: Indent,
    pub tab_width: usize,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Buffer {
            rope: Rope::new(),
            selections: Selections::single(Selection::caret(0)),
            history: History::new(),
            revision: 0,
            line_ending: LineEnding::Lf,
            bom: false,
            indent: Indent::Spaces(4),
            tab_width: 4,
        }
    }

    pub fn from_text(text: &str) -> Self {
        Buffer {
            rope: Rope::from_str(text),
            ..Buffer::new()
        }
    }

    pub fn from_decoded(decoded: &Decoded) -> Self {
        Buffer {
            rope: Rope::from_str(&decoded.text),
            line_ending: decoded.line_ending,
            bom: decoded.bom,
            ..Buffer::new()
        }
    }

    // --- Reading ------------------------------------------------------------------------------

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// The number of lines; a text ending in a newline has an empty last line, as in editors.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope
            .line_to_char(line.min(self.line_count().saturating_sub(1)))
    }

    pub fn line_end(&self, line: usize) -> usize {
        line_end_char(&self.rope, line.min(self.line_count().saturating_sub(1)))
    }

    /// The text of a line without its newline.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.line_count() {
            return String::new();
        }
        self.rope
            .slice(self.line_start(line)..self.line_end(line))
            .to_string()
    }

    pub fn line_of(&self, idx: usize) -> usize {
        self.rope.char_to_line(idx.min(self.len_chars()))
    }

    /// `(line, column in characters)`.
    pub fn line_col(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.len_chars());
        let line = self.rope.char_to_line(idx);
        (line, idx - self.rope.line_to_char(line))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        self.rope.slice(start..end).to_string()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, ending: LineEnding) {
        self.line_ending = ending;
    }

    pub fn bom(&self) -> bool {
        self.bom
    }

    pub fn is_dirty(&self) -> bool {
        !self.history.is_saved()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    pub fn primary(&self) -> Selection {
        self.selections.primary()
    }

    pub fn set_selections(&mut self, selections: Selections) {
        self.selections = selections;
        self.clamp_selections();
        self.history.seal();
    }

    pub fn set_cursor(&mut self, at: usize) {
        self.set_selections(Selections::single(Selection::caret(at)));
    }

    /// The text of every non-empty selection, joined by newlines; or, with `whole_lines`, the
    /// lines of the carets when nothing is selected (cut and copy of a bare caret take the line).
    pub fn selected_text(&self, whole_lines: bool) -> String {
        let parts: Vec<String> = self
            .selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| self.slice(s.range()))
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
        if whole_lines {
            let mut seen = Vec::new();
            let mut out = String::new();
            for s in self.selections.iter() {
                let line = self.line_of(s.head);
                if !seen.contains(&line) {
                    seen.push(line);
                    out.push_str(&self.line_text(line));
                    out.push('\n');
                }
            }
            return out;
        }
        String::new()
    }

    // --- Applying edits -----------------------------------------------------------------------

    /// Apply edits made from outside (a replace, a language server's fix). They may be in any
    /// order but must not overlap. The cursors move with the text. `None` when they do not fit.
    pub fn apply_edits(&mut self, edits: Vec<Edit>) -> Option<Vec<Change>> {
        let mut edits = edits;
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        let before = self.selections.clone();
        let mut after = before.clone();
        after.map(|s| Selection {
            anchor: map_through(s.anchor, &edits),
            head: map_through(s.head, &edits),
            goal: None,
        });
        self.commit(edits, before, after, false)
    }

    /// Replace the whole text as one undoable edit, keeping the cursors where they were.
    pub fn replace_all_text(&mut self, text: &str) -> Vec<Change> {
        if self.rope == text {
            return Vec::new();
        }
        let len = self.len_chars();
        let before = self.selections.clone();
        let after = before.clone();
        self.commit(vec![Edit::new(0..len, text)], before, after, false)
            .unwrap_or_default()
    }

    fn commit(
        &mut self,
        edits: Vec<Edit>,
        before: Selections,
        after: Selections,
        typing: bool,
    ) -> Option<Vec<Change>> {
        if edits.is_empty() {
            return Some(Vec::new());
        }
        let (inverse, changes) = edit::apply(&mut self.rope, &edits)?;
        self.revision += 1;
        self.selections = after;
        self.clamp_selections();
        self.history.record(
            Step {
                forward: edits,
                inverse,
            },
            before,
            self.selections.clone(),
            typing,
            Instant::now(),
        );
        Some(changes)
    }

    /// Run `f` on every selection to get the edit it makes: the range it replaces, the text, and
    /// where the caret lands within that text (in characters). Edits that would overlap are
    /// merged into one.
    fn edit_each(
        &mut self,
        typing: bool,
        mut f: impl FnMut(&Rope, Selection) -> Option<(Range<usize>, String, usize)>,
    ) -> Vec<Change> {
        let before = self.selections.clone();
        let mut edits: Vec<Edit> = Vec::with_capacity(before.len());
        // For each selection: the edit it ended up in, and where its caret lands in that edit's text.
        let mut carets: Vec<(usize, usize)> = Vec::with_capacity(before.len());
        for s in before.iter() {
            let (range, text, caret) =
                f(&self.rope, *s).unwrap_or_else(|| (s.head..s.head, String::new(), 0));
            if let Some(last) = edits.last_mut() {
                if range.start < last.range.end {
                    last.range.end = last.range.end.max(range.end);
                    last.text.push_str(&text);
                    let end_off = last.text.chars().count();
                    carets.push((edits.len() - 1, end_off));
                    continue;
                }
            }
            edits.push(Edit::new(range, text));
            carets.push((edits.len() - 1, caret));
        }
        let mut deltas = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for e in &edits {
            deltas.push(delta);
            delta += e.text.chars().count() as isize - (e.range.end - e.range.start) as isize;
        }
        let new_selections: Vec<Selection> = carets
            .iter()
            .map(|(i, off)| {
                let start = edits.get(*i).map_or(0, |e| e.range.start) as isize
                    + deltas.get(*i).copied().unwrap_or(0);
                Selection::caret(usize::try_from(start).unwrap_or(0) + off)
            })
            .collect();
        let primary = before
            .primary_index()
            .min(new_selections.len().saturating_sub(1));
        let after = Selections::from_vec(new_selections, primary);
        self.commit(edits, before, after, typing)
            .unwrap_or_default()
    }

    // --- Typing -------------------------------------------------------------------------------

    /// Insert text at every cursor, replacing the selections.
    pub fn insert_text(&mut self, text: &str) -> Vec<Change> {
        let n = text.chars().count();
        let typing = n == 1 && text != "\n";
        self.edit_each(typing, |_, s| Some((s.range(), text.to_owned(), n)))
    }

    /// Paste. With as many lines as cursors, each cursor gets one line. A whole line copied from
    /// a bare caret goes above the line of the caret it is pasted at.
    pub fn paste(&mut self, text: &str) -> Vec<Change> {
        let text = text.replace("\r\n", "\n");
        let count = self.selections.len();
        let lines: Vec<String> = text
            .trim_end_matches('\n')
            .split('\n')
            .map(str::to_owned)
            .collect();
        if count > 1 && lines.len() == count {
            let mut i = 0usize;
            return self.edit_each(false, |_, s| {
                let line = lines.get(i).cloned().unwrap_or_default();
                i += 1;
                let n = line.chars().count();
                Some((s.range(), line, n))
            });
        }
        if text.ends_with('\n') && lines.len() == 1 {
            let n = text.chars().count();
            return self.edit_each(false, |rope, s| {
                if !s.is_empty() {
                    return Some((s.range(), text.clone(), n));
                }
                let at = rope.line_to_char(rope.char_to_line(s.head));
                let col = s.head - at;
                Some((at..at, text.clone(), n + col))
            });
        }
        let n = text.chars().count();
        self.edit_each(false, |_, s| Some((s.range(), text.clone(), n)))
    }

    /// Type one character, with the editor's help: pairs close, a selection is surrounded, and a
    /// closer typed over one that is there steps over it.
    pub fn type_char(&mut self, c: char, pairs: Pairs) -> Vec<Change> {
        self.edit_each(true, |rope, s| {
            let len = rope.len_chars();
            let next = (s.end() < len).then(|| rope.char(s.end()));
            let prev = (s.start() > 0).then(|| rope.char(s.start() - 1));
            if !s.is_empty() {
                if let Some(close) = pairs.closing(c) {
                    let inner = rope.slice(s.range()).to_string();
                    let text = format!("{c}{inner}{close}");
                    let n = text.chars().count();
                    return Some((s.range(), text, n));
                }
                return Some((s.range(), c.to_string(), 1));
            }
            if pairs.is_closing(c) && next == Some(c) {
                return Some((s.head..s.head + 1, c.to_string(), 1));
            }
            if let Some(close) = pairs.closing(c) {
                let word_before = prev.is_some_and(motion::is_word_char);
                let next_ok = next.is_none_or(|n| {
                    n.is_whitespace() || pairs.is_closing(n) || n == ',' || n == ';'
                });
                let quote_ok = c != close || (!word_before && next != Some(c));
                if next_ok && quote_ok {
                    return Some((s.range(), format!("{c}{close}"), 1));
                }
            }
            Some((s.range(), c.to_string(), 1))
        })
    }

    /// Enter: a new line indented like the one above, one level deeper after an opening bracket.
    /// Between a bracket and its closer, the closer moves to a line of its own.
    pub fn newline(&mut self, rules: IndentRules) -> Vec<Change> {
        let unit = self.indent.unit();
        self.edit_each(false, |rope, s| {
            let line = rope.char_to_line(s.start());
            let line_start = rope.line_to_char(line);
            let lead: String = rope
                .slice(line_start..line_end_char(rope, line))
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let col = s.start() - line_start;
            // A caret inside the indentation keeps only the part of it before the caret.
            let base: String = lead.chars().take(col).collect();
            let before = rope.slice(line_start..s.start()).to_string();
            let last = before.trim_end().chars().last();
            let opens =
                last.is_some_and(|c| rules.open.contains(c)) || (rules.colon && last == Some(':'));
            let next = (s.end() < rope.len_chars()).then(|| rope.char(s.end()));
            let closes = opens && next.is_some_and(|n| matches!(n, '}' | ']' | ')'));
            if closes {
                let inner = format!("\n{base}{unit}");
                let caret = inner.chars().count();
                Some((s.range(), format!("{inner}\n{base}"), caret))
            } else {
                let extra = if opens { unit.as_str() } else { "" };
                let text = format!("\n{base}{extra}");
                let n = text.chars().count();
                Some((s.range(), text, n))
            }
        })
    }

    /// Backspace. Removes the selection; else the character before the caret (a whole pair when
    /// the caret is between them; one indent level when the caret is in the indentation).
    pub fn delete_backward(&mut self, pairs: Pairs) -> Vec<Change> {
        let indent = self.indent;
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            let at = s.head;
            if at == 0 {
                return None;
            }
            let prev = rope.char(at - 1);
            if let Some(close) = pairs.closing(prev) {
                if at < rope.len_chars() && rope.char(at) == close {
                    return Some((at - 1..at + 1, String::new(), 0));
                }
            }
            if let Indent::Spaces(n) = indent {
                let n = n.max(1);
                let line_start = rope.line_to_char(rope.char_to_line(at));
                let col = at - line_start;
                if col > 0 && rope.slice(line_start..at).chars().all(|c| c == ' ') {
                    let target = ((col - 1) / n) * n;
                    return Some((line_start + target..at, String::new(), 0));
                }
            }
            Some((motion::prev_grapheme(rope, at)..at, String::new(), 0))
        })
    }

    pub fn delete_forward(&mut self) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            if s.head >= rope.len_chars() {
                return None;
            }
            Some((
                s.head..motion::next_grapheme(rope, s.head),
                String::new(),
                0,
            ))
        })
    }

    pub fn delete_word_backward(&mut self) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            Some((
                motion::word_start_before(rope, s.head)..s.head,
                String::new(),
                0,
            ))
        })
    }

    pub fn delete_word_forward(&mut self) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            Some((
                s.head..motion::word_end_after(rope, s.head),
                String::new(),
                0,
            ))
        })
    }

    /// Delete from the caret to the end of its line (or the newline when it is already there).
    pub fn delete_to_line_end(&mut self) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            let end = line_end_char(rope, rope.char_to_line(s.head));
            let end = if end == s.head {
                (end + 1).min(rope.len_chars())
            } else {
                end
            };
            Some((s.head..end, String::new(), 0))
        })
    }

    pub fn delete_to_line_start(&mut self) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            if !s.is_empty() {
                return Some((s.range(), String::new(), 0));
            }
            let start = rope.line_to_char(rope.char_to_line(s.head));
            Some((start..s.head, String::new(), 0))
        })
    }

    // --- Lines --------------------------------------------------------------------------------

    /// The lines a selection touches. A selection that ends at the start of a line leaves that
    /// line out.
    fn touched_lines(&self, s: Selection) -> Range<usize> {
        let first = self.line_of(s.start());
        let mut last = self.line_of(s.end());
        if !s.is_empty() && last > first && s.end() == self.line_start(last) {
            last -= 1;
        }
        first..last + 1
    }

    /// All lines touched by the selections, each once, ascending.
    fn all_touched_lines(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = Vec::new();
        for s in self.selections.iter() {
            for l in self.touched_lines(*s) {
                if lines.last().is_none_or(|last| *last < l) {
                    lines.push(l);
                }
            }
        }
        lines
    }

    /// Tab: indent the lines of a multi-line selection, or insert an indent at the caret (to the
    /// next tab stop when indenting with spaces).
    pub fn indent(&mut self) -> Vec<Change> {
        let multi = self
            .selections
            .iter()
            .any(|s| !s.is_empty() && self.touched_lines(*s).len() > 1);
        if multi {
            let unit = self.indent.unit();
            let n = unit.chars().count();
            let before = self.selections.clone();
            let edits: Vec<Edit> = self
                .all_touched_lines()
                .into_iter()
                .filter(|l| self.line_end(*l) > self.line_start(*l))
                .map(|l| Edit::insert(self.line_start(l), unit.clone()))
                .collect();
            let starts: Vec<usize> = edits.iter().map(|e| e.range.start).collect();
            let mut after = before.clone();
            after.map(|s| {
                let forward = s.head >= s.anchor;
                Selection {
                    anchor: shifted(s.anchor, &starts, n, forward),
                    head: shifted(s.head, &starts, n, !forward),
                    goal: None,
                }
            });
            return self.commit(edits, before, after, false).unwrap_or_default();
        }
        let indent = self.indent;
        self.edit_each(false, |rope, s| {
            let text = match indent {
                Indent::Tabs => "\t".to_owned(),
                Indent::Spaces(n) => {
                    let n = n.max(1);
                    let col = motion::display_col(rope, s.start(), n);
                    " ".repeat(n - col % n)
                }
            };
            let len = text.chars().count();
            Some((s.range(), text, len))
        })
    }

    /// Shift+Tab: take one indent level off the start of every touched line.
    pub fn outdent(&mut self) -> Vec<Change> {
        let before = self.selections.clone();
        let width = match self.indent {
            Indent::Tabs => 1,
            Indent::Spaces(n) => n.max(1),
        };
        let mut edits = Vec::new();
        for line in self.all_touched_lines() {
            let start = self.line_start(line);
            let text = self.line_text(line);
            let removed = if text.starts_with('\t') {
                1
            } else {
                text.chars().take(width).take_while(|c| *c == ' ').count()
            };
            if removed > 0 {
                edits.push(Edit::delete(start..start + removed));
            }
        }
        if edits.is_empty() {
            return Vec::new();
        }
        let mut after = before.clone();
        after.map(|s| Selection {
            anchor: map_through(s.anchor, &edits),
            head: map_through(s.head, &edits),
            goal: None,
        });
        self.commit(edits, before, after, false).unwrap_or_default()
    }

    /// Alt+Up / Alt+Down: swap the touched lines with the line above or below.
    pub fn move_lines(&mut self, up: bool) -> Vec<Change> {
        let lines = self.all_touched_lines();
        let (Some(first), Some(last)) = (lines.first().copied(), lines.last().copied()) else {
            return Vec::new();
        };
        if (up && first == 0) || (!up && last + 1 >= self.line_count()) {
            return Vec::new();
        }
        let (from, to) = if up {
            (first - 1, last)
        } else {
            (first, last + 1)
        };
        let texts: Vec<String> = (from..=to).map(|l| self.line_text(l)).collect();
        let (moved, over) = if up {
            (texts.get(1..).unwrap_or(&[]), texts.first())
        } else {
            (texts.get(..texts.len() - 1).unwrap_or(&[]), texts.last())
        };
        let Some(over) = over else { return Vec::new() };
        let mut new_lines: Vec<&str> = Vec::with_capacity(texts.len());
        if up {
            new_lines.extend(moved.iter().map(String::as_str));
            new_lines.push(over);
        } else {
            new_lines.push(over);
            new_lines.extend(moved.iter().map(String::as_str));
        }
        let start = self.line_start(from);
        let end = self.line_end(to);
        let shift = over.chars().count() + 1;
        let before = self.selections.clone();
        let mut after = before.clone();
        after.map(|s| {
            let m = |x: usize| {
                if up {
                    x.saturating_sub(shift)
                } else {
                    x + shift
                }
            };
            Selection {
                anchor: m(s.anchor),
                head: m(s.head),
                goal: None,
            }
        });
        self.commit(
            vec![Edit::new(start..end, new_lines.join("\n"))],
            before,
            after,
            false,
        )
        .unwrap_or_default()
    }

    /// Shift+Alt+Up / Down: copy the touched lines above or below themselves.
    pub fn duplicate_lines(&mut self, up: bool) -> Vec<Change> {
        let lines = self.all_touched_lines();
        let (Some(first), Some(last)) = (lines.first().copied(), lines.last().copied()) else {
            return Vec::new();
        };
        let block: Vec<String> = (first..=last).map(|l| self.line_text(l)).collect();
        let block = block.join("\n");
        let n = block.chars().count() + 1;
        let before = self.selections.clone();
        let (edit, shift) = if up {
            (
                Edit::insert(self.line_start(first), format!("{block}\n")),
                0,
            )
        } else {
            (Edit::insert(self.line_end(last), format!("\n{block}")), n)
        };
        let mut after = before.clone();
        after.map(|s| Selection {
            anchor: s.anchor + shift,
            head: s.head + shift,
            goal: None,
        });
        self.commit(vec![edit], before, after, false)
            .unwrap_or_default()
    }

    /// Ctrl+Shift+K: delete the touched lines.
    pub fn delete_lines(&mut self) -> Vec<Change> {
        let lines = self.all_touched_lines();
        if lines.is_empty() {
            return Vec::new();
        }
        let before = self.selections.clone();
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for l in lines {
            match runs.last_mut() {
                Some((_, end)) if *end + 1 == l => *end = l,
                _ => runs.push((l, l)),
            }
        }
        let mut edits: Vec<Edit> = Vec::new();
        for (first, last) in runs {
            let mut start = self.line_start(first);
            let end = if last + 1 >= self.line_count() {
                // The last line: take the newline before it too, so no empty line is left over.
                start = start.saturating_sub(1);
                self.len_chars()
            } else {
                self.line_start(last + 1)
            };
            if let Some(previous) = edits.last() {
                if start < previous.range.end {
                    start = previous.range.end;
                }
            }
            edits.push(Edit::delete(start..end));
        }
        let mut after = before.clone();
        after.map(|s| Selection::caret(map_through(s.head, &edits)));
        let changes = self.commit(edits, before, after, false).unwrap_or_default();
        // The caret lands at the start of the line that took the deleted one's place.
        let carets: Vec<Selection> = self
            .selections
            .iter()
            .map(|s| Selection::caret(self.line_start(self.line_of(s.head))))
            .collect();
        let primary = self.selections.primary_index();
        self.selections = Selections::from_vec(carets, primary);
        changes
    }

    /// Ctrl+J: join each touched line with the next, with one space between.
    pub fn join_lines(&mut self) -> Vec<Change> {
        let before = self.selections.clone();
        let mut edits: Vec<Edit> = Vec::new();
        for s in before.iter() {
            let first = self.line_of(s.start());
            let mut last = self.line_of(s.end());
            if last == first {
                last = first + 1;
            }
            for line in first..last {
                if line + 1 >= self.line_count() {
                    break;
                }
                let end = self.line_end(line);
                let next_start = self.line_start(line + 1);
                let next_text = self.line_text(line + 1);
                let skip = next_text
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .count();
                let sep = if next_text.trim().is_empty() || self.line_text(line).is_empty() {
                    ""
                } else {
                    " "
                };
                if edits.last().is_none_or(|e| e.range.end <= end) {
                    edits.push(Edit::new(end..next_start + skip, sep));
                }
            }
        }
        if edits.is_empty() {
            return Vec::new();
        }
        let mut after = before.clone();
        after.map(|s| Selection::caret(map_through(s.head, &edits)));
        self.commit(edits, before, after, false).unwrap_or_default()
    }

    /// Ctrl+Enter / Ctrl+Shift+Enter: open an empty line, indented, below or above.
    pub fn open_line(&mut self, below: bool) -> Vec<Change> {
        self.edit_each(false, |rope, s| {
            let line = rope.char_to_line(s.head);
            let lead: String = rope
                .slice(rope.line_to_char(line)..line_end_char(rope, line))
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if below {
                let at = line_end_char(rope, line);
                let text = format!("\n{lead}");
                let n = text.chars().count();
                Some((at..at, text, n))
            } else {
                let at = rope.line_to_char(line);
                let n = lead.chars().count();
                Some((at..at, format!("{lead}\n"), n))
            }
        })
    }

    /// Ctrl+/: comment or uncomment the touched lines with `token` (`//`, `#`...).
    pub fn toggle_line_comment(&mut self, token: &str) -> Vec<Change> {
        if token.is_empty() {
            return Vec::new();
        }
        let lines: Vec<usize> = self
            .all_touched_lines()
            .into_iter()
            .filter(|l| !self.line_text(*l).trim().is_empty())
            .collect();
        if lines.is_empty() {
            return Vec::new();
        }
        let before = self.selections.clone();
        let all_commented = lines
            .iter()
            .all(|l| self.line_text(*l).trim_start().starts_with(token));
        let margin = lines
            .iter()
            .map(|l| {
                self.line_text(*l)
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .count()
            })
            .min()
            .unwrap_or(0);
        let token_len = token.chars().count();
        let mut edits = Vec::new();
        for l in lines {
            let start = self.line_start(l);
            let text = self.line_text(l);
            if all_commented {
                let lead = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                let space = usize::from(text.chars().nth(lead + token_len) == Some(' '));
                edits.push(Edit::delete(start + lead..start + lead + token_len + space));
            } else {
                edits.push(Edit::insert(start + margin, format!("{token} ")));
            }
        }
        let mut after = before.clone();
        after.map(|s| Selection {
            anchor: map_through(s.anchor, &edits),
            head: map_through(s.head, &edits),
            goal: None,
        });
        self.commit(edits, before, after, false).unwrap_or_default()
    }

    // --- Moving the cursors -------------------------------------------------------------------

    pub fn move_cursors(&mut self, motion: Motion, extend: bool) {
        let tab = self.tab_width;
        let rope = &self.rope;
        let last_line = rope.len_lines().saturating_sub(1);
        let vertical = matches!(
            motion,
            Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_)
        );
        let mut moved = Vec::new();
        for s in self.selections.iter() {
            let head = s.head;
            let mut goal = s.goal;
            let new_head = match motion {
                Motion::Left if !extend && !s.is_empty() => s.start(),
                Motion::Right if !extend && !s.is_empty() => s.end(),
                Motion::Left => motion::prev_grapheme(rope, head),
                Motion::Right => motion::next_grapheme(rope, head),
                Motion::WordLeft => motion::word_start_before(rope, head),
                Motion::WordRight => motion::word_end_after(rope, head),
                Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) => {
                    let line = rope.char_to_line(head);
                    let col = goal.unwrap_or_else(|| motion::display_col(rope, head, tab));
                    goal = Some(col);
                    let target = match motion {
                        Motion::Up => line.checked_sub(1),
                        Motion::Down => (line < last_line).then_some(line + 1),
                        Motion::PageUp(n) => (line > 0).then(|| line.saturating_sub(n.max(1))),
                        Motion::PageDown(n) => {
                            (line < last_line).then(|| (line + n.max(1)).min(last_line))
                        }
                        _ => None,
                    };
                    match target {
                        Some(t) => motion::offset_at_display_col(rope, t, col, tab),
                        None if matches!(motion, Motion::Up | Motion::PageUp(_)) => 0,
                        None => rope.len_chars(),
                    }
                }
                Motion::LineStart => {
                    let line = rope.char_to_line(head);
                    let start = rope.line_to_char(line);
                    let first = start
                        + rope
                            .slice(start..line_end_char(rope, line))
                            .chars()
                            .take_while(|c| *c == ' ' || *c == '\t')
                            .count();
                    if head == first {
                        start
                    } else {
                        first
                    }
                }
                Motion::LineEnd => line_end_char(rope, rope.char_to_line(head)),
                Motion::DocStart => 0,
                Motion::DocEnd => rope.len_chars(),
                Motion::ParagraphUp | Motion::ParagraphDown => {
                    let blank = |l: usize| line_end_char(rope, l) == rope.line_to_char(l);
                    let mut line = rope.char_to_line(head);
                    if matches!(motion, Motion::ParagraphUp) {
                        while line > 0 && blank(line) {
                            line -= 1;
                        }
                        while line > 0 && !blank(line) {
                            line -= 1;
                        }
                        rope.line_to_char(line)
                    } else {
                        while line < last_line && blank(line) {
                            line += 1;
                        }
                        while line < last_line && !blank(line) {
                            line += 1;
                        }
                        if line >= last_line && !blank(line) {
                            rope.len_chars()
                        } else {
                            rope.line_to_char(line)
                        }
                    }
                }
            };
            moved.push(Selection {
                anchor: if extend { s.anchor } else { new_head },
                head: new_head,
                goal: if vertical { goal } else { None },
            });
        }
        let primary = self.selections.primary_index();
        self.selections = Selections::from_vec(moved, primary);
        self.history.seal();
    }

    pub fn select_all(&mut self) {
        self.selections = Selections::single(Selection::new(0, self.len_chars()));
        self.history.seal();
    }

    /// Expand each selection to the whole lines it touches.
    pub fn select_lines(&mut self) {
        let sel: Vec<Selection> = self
            .selections
            .iter()
            .map(|s| {
                let r = self.touched_lines(*s);
                let start = self.line_start(r.start);
                let end = if r.end >= self.line_count() {
                    self.len_chars()
                } else {
                    self.line_start(r.end)
                };
                Selection::new(start, end)
            })
            .collect();
        let primary = self.selections.primary_index();
        self.selections = Selections::from_vec(sel, primary);
        self.history.seal();
    }

    pub fn select_word_at(&mut self, idx: usize) {
        let (a, b) = motion::word_range_at(&self.rope, idx);
        self.selections = Selections::single(Selection::new(a, b));
        self.history.seal();
    }

    pub fn select_line_at(&mut self, idx: usize) {
        let line = self.line_of(idx);
        let start = self.line_start(line);
        let end = if line + 1 >= self.line_count() {
            self.len_chars()
        } else {
            self.line_start(line + 1)
        };
        self.selections = Selections::single(Selection::new(start, end));
        self.history.seal();
    }

    /// Shift+click: extend the primary selection to `idx`.
    pub fn extend_to(&mut self, idx: usize) {
        let mut sel = self.selections.primary();
        sel.head = idx.min(self.len_chars());
        sel.goal = None;
        self.selections = Selections::single(sel);
        self.history.seal();
    }

    /// Alt+click: another cursor.
    pub fn add_cursor(&mut self, idx: usize) {
        let mut selections = self.selections.clone();
        selections.push(Selection::caret(idx.min(self.len_chars())));
        self.selections = selections;
        self.history.seal();
    }

    /// Ctrl+Alt+Up / Down: a cursor above or below each one.
    pub fn add_cursor_vertical(&mut self, up: bool) {
        let tab = self.tab_width;
        let last_line = self.line_count().saturating_sub(1);
        let mut added = Vec::new();
        for s in self.selections.iter() {
            let line = self.line_of(s.head);
            let col = s
                .goal
                .unwrap_or_else(|| motion::display_col(&self.rope, s.head, tab));
            let target = if up {
                line.checked_sub(1)
            } else {
                (line < last_line).then_some(line + 1)
            };
            if let Some(t) = target {
                let at = motion::offset_at_display_col(&self.rope, t, col, tab);
                added.push(Selection {
                    anchor: at,
                    head: at,
                    goal: Some(col),
                });
            }
        }
        if added.is_empty() {
            return;
        }
        let mut all: Vec<Selection> = self.selections.iter().copied().collect();
        let primary = if up {
            all.len()
        } else {
            all.len() + added.len() - 1
        };
        all.extend(added);
        self.selections = Selections::from_vec(all, primary);
        self.history.seal();
    }

    /// Ctrl+D: select the word at the caret; again, add the next occurrence of the selection.
    pub fn select_next_occurrence(&mut self) {
        let primary = self.selections.primary();
        if primary.is_empty() {
            let (a, b) = motion::word_range_at(&self.rope, primary.head);
            if a != b {
                self.selections = Selections::single(Selection::new(a, b));
                self.history.seal();
            }
            return;
        }
        let needle = self.slice(primary.range());
        if needle.is_empty() {
            return;
        }
        let text = self.text();
        let from_byte = self.rope.char_to_byte(primary.end());
        let found = text
            .get(from_byte..)
            .and_then(|rest| rest.find(&needle))
            .map(|b| from_byte + b)
            .or_else(|| text.find(&needle));
        let Some(byte) = found else { return };
        let start = self.rope.byte_to_char(byte);
        let end = start + needle.chars().count();
        if self
            .selections
            .iter()
            .any(|s| s.start() == start && s.end() == end)
        {
            return;
        }
        let mut selections = self.selections.clone();
        selections.push(Selection::new(start, end));
        self.selections = selections;
        self.history.seal();
    }

    /// Ctrl+Shift+L: select every occurrence of the selection (or of the word at the caret).
    pub fn select_all_occurrences(&mut self) {
        let primary = self.selections.primary();
        let range = if primary.is_empty() {
            let (a, b) = motion::word_range_at(&self.rope, primary.head);
            a..b
        } else {
            primary.range()
        };
        let needle = self.slice(range);
        if needle.is_empty() {
            return;
        }
        let text = self.text();
        let mut found = Vec::new();
        let mut from = 0usize;
        while let Some(pos) = text.get(from..).and_then(|rest| rest.find(&needle)) {
            let byte = from + pos;
            let start = self.rope.byte_to_char(byte);
            found.push(Selection::new(start, start + needle.chars().count()));
            from = byte + needle.len();
            if found.len() >= 10_000 {
                break;
            }
        }
        if !found.is_empty() {
            self.selections = Selections::from_vec(found, 0);
            self.history.seal();
        }
    }

    /// Escape: back to one caret.
    pub fn collapse_selections(&mut self) {
        let p = self.selections.primary();
        self.selections = Selections::single(Selection::caret(p.head));
        self.history.seal();
    }

    // --- History ------------------------------------------------------------------------------

    pub fn undo(&mut self) -> Vec<Change> {
        let Some(tx) = self.history.pop_undo() else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        for step in tx.steps.iter().rev() {
            if let Some((_, c)) = edit::apply(&mut self.rope, &step.inverse) {
                changes.extend(c);
            }
        }
        self.revision += 1;
        self.selections = tx.before.clone();
        self.clamp_selections();
        self.history.push_undone(tx);
        changes
    }

    pub fn redo(&mut self) -> Vec<Change> {
        let Some(tx) = self.history.pop_redo() else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        for step in &tx.steps {
            if let Some((_, c)) = edit::apply(&mut self.rope, &step.forward) {
                changes.extend(c);
            }
        }
        self.revision += 1;
        self.selections = tx.after.clone();
        self.clamp_selections();
        self.history.push_done(tx);
        changes
    }

    fn clamp_selections(&mut self) {
        let len = self.len_chars();
        self.selections.map(|s| Selection {
            anchor: s.anchor.min(len),
            head: s.head.min(len),
            goal: s.goal,
        });
    }
}

/// Where `offset` ends up after `edits` (sorted, not overlapping): later text shifts; a point in a
/// replaced range moves to the end of its replacement.
pub fn map_through(offset: usize, edits: &[Edit]) -> usize {
    let mut delta: isize = 0;
    for e in edits {
        if offset >= e.range.end {
            delta += e.text.chars().count() as isize - (e.range.end - e.range.start) as isize;
        } else if offset > e.range.start {
            let start = usize::try_from(e.range.start as isize + delta).unwrap_or(0);
            return start + e.text.chars().count();
        } else {
            break;
        }
    }
    usize::try_from(offset as isize + delta).unwrap_or(0)
}

/// Move `offset` for `n` characters inserted at each of `starts`. An offset exactly at an
/// insertion point moves too, unless it is the start of a selection (`stays`), so a selection
/// that began at a line start still covers the text it indented.
fn shifted(offset: usize, starts: &[usize], n: usize, stays: bool) -> usize {
    let count = starts
        .iter()
        .filter(|s| if stays { **s < offset } else { **s <= offset })
        .count();
    offset + count * n
}

#[cfg(test)]
mod tests;
