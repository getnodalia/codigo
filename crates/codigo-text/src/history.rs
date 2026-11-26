// SPDX-License-Identifier: MIT OR Apache-2.0
//! Undo and redo.

use std::time::{Duration, Instant};

use crate::edit::Edit;
use crate::selection::Selections;

/// Edits within this long of each other, when they are typing, undo together.
const COALESCE: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug)]
pub struct Step {
    pub forward: Vec<Edit>,
    pub inverse: Vec<Edit>,
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub steps: Vec<Step>,
    pub before: Selections,
    pub after: Selections,
    typing: bool,
    at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    done: Vec<Transaction>,
    undone: Vec<Transaction>,
    /// What `done.len()` was when the text was last saved (`None` once that state is gone).
    saved: Option<usize>,
    /// A save closes the open typing run, so later typing cannot merge into a saved state.
    sealed: bool,
}

impl History {
    pub fn new() -> Self {
        History {
            saved: Some(0),
            ..History::default()
        }
    }

    /// Record an edit. `typing` edits that follow a typing edit closely, from where it left off,
    /// extend it instead of starting a new undo step.
    pub fn record(
        &mut self,
        step: Step,
        before: Selections,
        after: Selections,
        typing: bool,
        now: Instant,
    ) {
        if self.saved.is_some_and(|s| s > self.done.len()) {
            self.saved = None;
        }
        self.undone.clear();
        if typing && !self.sealed {
            if let Some(last) = self.done.last_mut() {
                if last.typing && now.duration_since(last.at) < COALESCE && last.after == before {
                    last.steps.push(step);
                    last.after = after;
                    last.at = now;
                    return;
                }
            }
        }
        self.sealed = false;
        self.done.push(Transaction {
            steps: vec![step],
            before,
            after,
            typing,
            at: now,
        });
    }

    /// Stop the current typing run from absorbing what comes next.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.done.len());
        self.sealed = true;
    }

    pub fn is_saved(&self) -> bool {
        self.saved == Some(self.done.len())
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn pop_undo(&mut self) -> Option<Transaction> {
        let tx = self.done.pop()?;
        self.sealed = true;
        Some(tx)
    }

    pub fn push_undone(&mut self, tx: Transaction) {
        self.undone.push(tx);
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.undone.pop()
    }

    pub fn push_done(&mut self, tx: Transaction) {
        self.done.push(tx);
        self.sealed = true;
    }
}
