// SPDX-License-Identifier: MIT OR Apache-2.0
//! Snippets in the editor. Tab after a snippet's word expands it and selects its first place to
//! fill in; Tab moves on to the next, Shift+Tab goes back, and the last Tab (or leaving the
//! snippet) ends it. While a place is being typed in, the places after it move with the text.

use std::path::PathBuf;
use std::time::SystemTime;

use codigo_engine::snippets::{self, UserSnippets};
use codigo_engine::{paths, DocId};
use codigo_text::Edit;
use egui::{Key, Modifiers};

use crate::app::App;
use crate::i18n::tr;

struct Session {
    doc: DocId,
    /// Char ranges of the places, in the order they are visited.
    stops: Vec<(usize, usize)>,
    current: usize,
    revision: u64,
    len: usize,
}

pub struct State {
    user: UserSnippets,
    loaded: Option<SystemTime>,
    session: Option<Session>,
}

impl State {
    pub fn new() -> State {
        let mut state = State {
            user: UserSnippets::default(),
            loaded: None,
            session: None,
        };
        state.reload();
        state
    }

    /// Read the person's snippets again when the file changed.
    fn reload(&mut self) {
        let Some(path) = file() else {
            return;
        };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified == self.loaded && self.loaded.is_some() {
            return;
        }
        self.loaded = modified;
        self.user = path.parent().map(UserSnippets::load).unwrap_or_default();
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

fn file() -> Option<PathBuf> {
    paths::config_dir().map(|d| d.join("snippets.json"))
}

const EXAMPLE: &str = r#"{
  "rust": {
    "Print a value": {
      "prefix": "pv",
      "body": ["println!(\"${1:value} = {:?}\", ${1:value});", "$0"]
    }
  }
}
"#;

/// Open the person's snippets file, creating it with an example when there is none.
pub fn open_user_file(app: &mut App) {
    let Some(path) = file() else {
        app.toast(tr("The settings folder could not be found."));
        return;
    };
    if !path.exists() {
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, EXAMPLE));
        if written.is_err() {
            app.toast(tr("The snippets file could not be created."));
            return;
        }
    }
    app.open_path(&path, None, true);
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn select(app: &mut App, start: usize, end: usize) {
    if let Some(tab) = app.tabs.get_mut(app.active) {
        tab.doc.navigate(|b| {
            b.set_cursor(start);
            if end > start {
                b.extend_to(end);
            }
        });
        tab.view.reveal = true;
    }
}

/// Tab, Shift+Tab. Call before the shortcuts see the keyboard, and after a completion popup has
/// had its turn: a snippet only expands when nothing else wanted the key.
pub fn keys(app: &mut App, ctx: &egui::Context, editor_focus: bool) {
    if !editor_focus || !matches!(app.overlay, crate::overlays::Overlay::None) {
        return;
    }
    if matches!(app.popup, crate::lsp_ui::Popup::Completion(_)) {
        return;
    }
    if app.snippets.session.is_some() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab)) {
            move_to(app, false);
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            move_to(app, true);
        }
        return;
    }
    let pressed = ctx.input(|i| i.key_pressed(Key::Tab) && !i.modifiers.any());
    if pressed && expand_at_caret(app) {
        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
    }
}

fn move_to(app: &mut App, forward: bool) {
    let Some(session) = app.snippets.session.as_mut() else {
        return;
    };
    let last = session.stops.len().saturating_sub(1);
    let next = if forward {
        (session.current + 1).min(last)
    } else {
        session.current.saturating_sub(1)
    };
    session.current = next;
    let Some(&(start, end)) = session.stops.get(next) else {
        app.snippets.session = None;
        return;
    };
    if forward && next == last {
        app.snippets.session = None;
    }
    select(app, start, end);
}

/// Expand the snippet named by the word just before the caret. `false` when there is none, so
/// the Tab can do its usual job.
fn expand_at_caret(app: &mut App) -> bool {
    app.snippets.reload();
    let Some(tab) = app.tabs.get(app.active) else {
        return false;
    };
    let b = tab.doc.buffer();
    if b.selections().len() > 1 {
        return false;
    }
    let sel = b.primary();
    if sel.anchor != sel.head {
        return false;
    }
    let head = sel.head;
    let mut anchor = head;
    while anchor > 0 && b.slice(anchor - 1..anchor).chars().all(is_word) {
        anchor -= 1;
    }
    let after_is_word = head < b.len_chars() && b.slice(head..head + 1).chars().all(is_word);
    let before = (anchor > 0).then(|| b.slice(anchor - 1..anchor));
    if anchor == head || after_is_word || before.as_deref() == Some(".") {
        return false;
    }
    let word = b.slice(anchor..head);
    let Some(snippet) = snippets::find(tab.doc.language().id, &word, &app.snippets.user) else {
        return false;
    };
    let line = b.line_of(anchor);
    let indent: String = b
        .line_text(line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let expansion = snippets::expand(&snippet.body, &indent, &b.indent.unit());
    let doc = tab.doc.id();
    let Some(tab) = app.tabs.get_mut(app.active) else {
        return false;
    };
    let text = expansion.text.clone();
    tab.doc.apply(|b| {
        b.apply_edits(vec![Edit::new(anchor..head, text)])
            .unwrap_or_default()
    });
    let stops: Vec<(usize, usize)> = expansion
        .stops
        .iter()
        .map(|(s, e)| (anchor + s, anchor + e))
        .collect();
    let (revision, len) = (tab.doc.buffer().revision(), tab.doc.buffer().len_chars());
    let Some(&(start, end)) = stops.first() else {
        return true;
    };
    select(app, start, end);
    if stops.len() > 1 {
        app.snippets.session = Some(Session {
            doc,
            stops,
            current: 0,
            revision,
            len,
        });
    }
    true
}

/// Keep the session honest after each frame: move the later places with the text, and end it
/// when the caret has left the place being filled in.
pub fn update(app: &mut App) {
    let Some(session) = app.snippets.session.as_mut() else {
        return;
    };
    let Some(tab) = app
        .tabs
        .get(app.active)
        .filter(|t| t.doc.id() == session.doc)
    else {
        app.snippets.session = None;
        return;
    };
    let b = tab.doc.buffer();
    if b.revision() != session.revision {
        let delta = b.len_chars() as i64 - session.len as i64;
        let current = session.current;
        let shift = |v: usize| usize::try_from((v as i64).saturating_add(delta)).ok();
        let mut ok = true;
        for (n, stop) in session.stops.iter_mut().enumerate() {
            if n < current {
                continue;
            }
            if n == current {
                match shift(stop.1) {
                    Some(end) if end >= stop.0 => stop.1 = end,
                    _ => ok = false,
                }
            } else {
                match (shift(stop.0), shift(stop.1)) {
                    (Some(s), Some(e)) => *stop = (s, e),
                    _ => ok = false,
                }
            }
        }
        session.revision = b.revision();
        session.len = b.len_chars();
        if !ok {
            app.snippets.session = None;
            return;
        }
    }
    let head = b.primary().head;
    let inside = session
        .stops
        .get(session.current)
        .is_some_and(|&(s, e)| head >= s && head <= e);
    if !inside || b.selections().len() > 1 {
        app.snippets.session = None;
    }
}
