// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything the user can ask the editor to do, as data.
//!
//! The menu bar, the command palette, the keyboard and agents all work from this one table, so a
//! command added here shows up in all of them and a shortcut cannot be bound twice.

/// A key, independent of any toolkit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Plus,
    Minus,
    Comma,
    Period,
    Slash,
    Backslash,
    Backtick,
    OpenBracket,
    CloseBracket,
    Semicolon,
    Quote,
}

impl Key {
    /// How the key is written on a keycap.
    pub fn label(self) -> &'static str {
        use Key::*;
        match self {
            A => "A",
            B => "B",
            C => "C",
            D => "D",
            E => "E",
            F => "F",
            G => "G",
            H => "H",
            I => "I",
            J => "J",
            K => "K",
            L => "L",
            M => "M",
            N => "N",
            O => "O",
            P => "P",
            Q => "Q",
            R => "R",
            S => "S",
            T => "T",
            U => "U",
            V => "V",
            W => "W",
            X => "X",
            Y => "Y",
            Z => "Z",
            Num0 => "0",
            Num1 => "1",
            Num2 => "2",
            Num3 => "3",
            Num4 => "4",
            Num5 => "5",
            Num6 => "6",
            Num7 => "7",
            Num8 => "8",
            Num9 => "9",
            F1 => "F1",
            F2 => "F2",
            F3 => "F3",
            F4 => "F4",
            F5 => "F5",
            F6 => "F6",
            F7 => "F7",
            F8 => "F8",
            F9 => "F9",
            F10 => "F10",
            F11 => "F11",
            F12 => "F12",
            Enter => "Enter",
            Escape => "Esc",
            Tab => "Tab",
            Backspace => "Backspace",
            Delete => "Del",
            Space => "Space",
            Up => "↑",
            Down => "↓",
            Left => "←",
            Right => "→",
            Home => "Home",
            End => "End",
            PageUp => "PgUp",
            PageDown => "PgDn",
            Plus => "+",
            Minus => "-",
            Comma => ",",
            Period => ".",
            Slash => "/",
            Backslash => "\\",
            Backtick => "`",
            OpenBracket => "[",
            CloseBracket => "]",
            Semicolon => ";",
            Quote => "'",
        }
    }
}

/// A key with modifiers. `primary` is Ctrl on Linux and Windows and Command on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    pub const fn new(primary: bool, shift: bool, alt: bool, key: Key) -> Self {
        Self {
            primary,
            shift,
            alt,
            key,
        }
    }

    /// `Ctrl+Shift+P`, or `⇧⌘P` with `mac`.
    pub fn label(&self, mac: bool) -> String {
        let mut out = String::new();
        if mac {
            if self.alt {
                out.push('⌥');
            }
            if self.shift {
                out.push('⇧');
            }
            if self.primary {
                out.push('⌘');
            }
            out.push_str(self.key.label());
        } else {
            if self.primary {
                out.push_str("Ctrl+");
            }
            if self.alt {
                out.push_str("Alt+");
            }
            if self.shift {
                out.push_str("Shift+");
            }
            out.push_str(self.key.label());
        }
        out
    }
}

const fn ctrl(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(true, false, false, key))
}
const fn ctrl_shift(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(true, true, false, key))
}
const fn ctrl_alt(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(true, false, true, key))
}
const fn alt(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(false, false, true, key))
}
const fn alt_shift(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(false, true, true, key))
}
const fn shift(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(false, true, false, key))
}
const fn plain(key: Key) -> Option<Shortcut> {
    Some(Shortcut::new(false, false, false, key))
}
const NONE: Option<Shortcut> = None;

/// Where a command sits in the menu bar and the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    File,
    Edit,
    Selection,
    View,
    Go,
    Run,
    Help,
}

impl Category {
    /// English name; the interface translates it.
    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Edit => "Edit",
            Category::Selection => "Selection",
            Category::View => "View",
            Category::Go => "Go",
            Category::Run => "Run",
            Category::Help => "Help",
        }
    }
}

macro_rules! commands {
    ($($variant:ident => ($id:literal, $label:literal, $cat:ident, $shortcut:expr)),* $(,)?) => {
        /// A command. The id is stable (settings and keybindings refer to it), the label is the
        /// English text the interface translates.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Cmd { $($variant),* }

        impl Cmd {
            pub const ALL: &'static [Cmd] = &[$(Cmd::$variant),*];

            pub fn id(self) -> &'static str {
                match self { $(Cmd::$variant => $id),* }
            }
            pub fn label(self) -> &'static str {
                match self { $(Cmd::$variant => $label),* }
            }
            pub fn category(self) -> Category {
                match self { $(Cmd::$variant => Category::$cat),* }
            }
            pub fn shortcut(self) -> Option<Shortcut> {
                match self { $(Cmd::$variant => $shortcut),* }
            }
        }
    };
}

commands! {
    NewFile => ("file.new", "New file", File, ctrl(Key::N)),
    OpenFile => ("file.open", "Open file…", File, ctrl(Key::O)),
    OpenFolder => ("file.openFolder", "Open folder…", File, ctrl_shift(Key::O)),
    Save => ("file.save", "Save", File, ctrl(Key::S)),
    SaveAs => ("file.saveAs", "Save as…", File, ctrl_shift(Key::S)),
    SaveAll => ("file.saveAll", "Save all", File, ctrl_alt(Key::S)),
    CloseTab => ("file.close", "Close tab", File, ctrl(Key::W)),
    CloseOtherTabs => ("file.closeOthers", "Close other tabs", File, NONE),
    CloseAllTabs => ("file.closeAll", "Close all tabs", File, ctrl_shift(Key::W)),
    ReopenClosedTab => ("file.reopenClosed", "Reopen closed tab", File, ctrl_shift(Key::T)),
    RevealInFiles => ("file.reveal", "Show in file manager", File, NONE),
    Quit => ("file.quit", "Quit", File, ctrl(Key::Q)),

    Undo => ("edit.undo", "Undo", Edit, ctrl(Key::Z)),
    Redo => ("edit.redo", "Redo", Edit, ctrl_shift(Key::Z)),
    Cut => ("edit.cut", "Cut", Edit, ctrl(Key::X)),
    Copy => ("edit.copy", "Copy", Edit, ctrl(Key::C)),
    Paste => ("edit.paste", "Paste", Edit, ctrl(Key::V)),
    Find => ("edit.find", "Find", Edit, ctrl(Key::F)),
    Replace => ("edit.replace", "Replace", Edit, ctrl(Key::H)),
    FindNext => ("edit.findNext", "Find next", Edit, plain(Key::F3)),
    FindPrevious => ("edit.findPrevious", "Find previous", Edit, Some(Shortcut::new(false, true, false, Key::F3))),
    FindInProject => ("edit.findInProject", "Search in project", Edit, ctrl_shift(Key::F)),
    ToggleLineComment => ("edit.toggleComment", "Toggle line comment", Edit, ctrl(Key::Slash)),
    Indent => ("edit.indent", "Indent", Edit, ctrl(Key::CloseBracket)),
    Outdent => ("edit.outdent", "Outdent", Edit, ctrl(Key::OpenBracket)),
    MoveLineUp => ("edit.moveLineUp", "Move line up", Edit, alt(Key::Up)),
    MoveLineDown => ("edit.moveLineDown", "Move line down", Edit, alt(Key::Down)),
    DuplicateLineUp => ("edit.duplicateLineUp", "Duplicate line up", Edit, alt_shift(Key::Up)),
    DuplicateLineDown => ("edit.duplicateLineDown", "Duplicate line down", Edit, alt_shift(Key::Down)),
    DeleteLine => ("edit.deleteLine", "Delete line", Edit, ctrl_shift(Key::K)),
    JoinLines => ("edit.joinLines", "Join lines", Edit, ctrl(Key::J)),
    TriggerCompletion => ("edit.complete", "Suggest completions", Edit, ctrl(Key::Space)),
    OpenLineBelow => ("edit.openLineBelow", "Insert line below", Edit, ctrl(Key::Enter)),
    OpenLineAbove => ("edit.openLineAbove", "Insert line above", Edit, Some(Shortcut::new(true, true, false, Key::Enter))),
    TrimTrailingWhitespace => ("edit.trimWhitespace", "Trim trailing whitespace", Edit, NONE),

    SelectAll => ("select.all", "Select all", Selection, ctrl(Key::A)),
    SelectLine => ("select.line", "Select line", Selection, ctrl(Key::L)),
    SelectNextOccurrence => ("select.nextOccurrence", "Add next occurrence", Selection, ctrl(Key::D)),
    SelectAllOccurrences => ("select.allOccurrences", "Select all occurrences", Selection, ctrl_shift(Key::L)),
    AddCursorAbove => ("select.cursorAbove", "Add cursor above", Selection, ctrl_alt(Key::Up)),
    AddCursorBelow => ("select.cursorBelow", "Add cursor below", Selection, ctrl_alt(Key::Down)),
    CollapseSelections => ("select.collapse", "Single cursor", Selection, plain(Key::Escape)),

    CommandPalette => ("view.commandPalette", "Command palette…", View, ctrl_shift(Key::P)),
    ToggleSidebar => ("view.toggleSidebar", "Show or hide the sidebar", View, ctrl(Key::B)),
    ToggleTerminal => ("view.toggleTerminal", "Show or hide the terminal", View, ctrl(Key::Backtick)),
    NewTerminal => ("terminal.new", "New terminal", View, ctrl_shift(Key::Backtick)),
    CloseTerminal => ("terminal.close", "Close the terminal", View, NONE),
    ToggleWordWrap => ("view.toggleWordWrap", "Word wrap", View, alt(Key::Z)),
    ToggleWhitespace => ("view.toggleWhitespace", "Show whitespace", View, NONE),
    ZoomIn => ("view.zoomIn", "Zoom in", View, ctrl(Key::Plus)),
    ZoomOut => ("view.zoomOut", "Zoom out", View, ctrl(Key::Minus)),
    ZoomReset => ("view.zoomReset", "Reset zoom", View, ctrl(Key::Num0)),
    FocusEditor => ("view.focusEditor", "Focus editor", View, NONE),
    FocusExplorer => ("view.focusExplorer", "Focus explorer", View, ctrl_shift(Key::E)),
    ChangeLanguage => ("view.changeLanguage", "Change language mode…", View, NONE),

    RunProject => ("run.start", "Run the project", Run, plain(Key::F5)),
    BuildProject => ("run.build", "Build the project", Run, ctrl_shift(Key::B)),
    TestProject => ("run.test", "Run the tests", Run, NONE),
    CheckProject => ("run.check", "Check the code", Run, NONE),
    StopRun => ("run.stop", "Stop the running program", Run, shift(Key::F5)),

    QuickOpen => ("go.quickOpen", "Go to file…", Go, ctrl(Key::P)),
    GoToLine => ("go.line", "Go to line…", Go, ctrl(Key::G)),
    GoToDefinition => ("go.definition", "Go to definition", Go, plain(Key::F12)),
    ShowHover => ("go.hover", "Show information about the symbol", Go, ctrl_shift(Key::Space)),
    NextTab => ("go.nextTab", "Next tab", Go, Some(Shortcut::new(true, false, false, Key::PageDown))),
    PreviousTab => ("go.previousTab", "Previous tab", Go, Some(Shortcut::new(true, false, false, Key::PageUp))),

    Preferences => ("app.preferences", "Preferences", Help, ctrl(Key::Comma)),
    KeyboardShortcuts => ("app.shortcuts", "Keyboard shortcuts", Help, plain(Key::F1)),
    About => ("app.about", "About Código", Help, NONE),
}

impl Cmd {
    pub fn from_id(id: &str) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|c| c.id() == id)
    }

    /// The command bound to a shortcut, if any.
    pub fn for_shortcut(shortcut: Shortcut) -> Option<Cmd> {
        Cmd::ALL
            .iter()
            .copied()
            .find(|c| c.shortcut() == Some(shortcut))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_round_trip() {
        let mut seen = HashSet::new();
        for c in Cmd::ALL {
            assert!(seen.insert(c.id()), "duplicate id {}", c.id());
            assert_eq!(Cmd::from_id(c.id()), Some(*c));
        }
        assert_eq!(Cmd::from_id("no.such"), None);
    }

    #[test]
    fn a_shortcut_is_bound_once() {
        let mut seen = HashSet::new();
        for c in Cmd::ALL {
            if let Some(s) = c.shortcut() {
                assert!(seen.insert(s), "{} reuses {}", c.id(), s.label(false));
                assert_eq!(Cmd::for_shortcut(s), Some(*c));
            }
        }
    }

    #[test]
    fn shortcuts_read_like_keycaps() {
        let p = Cmd::CommandPalette.shortcut().expect("palette has one");
        assert_eq!(p.label(false), "Ctrl+Shift+P");
        assert_eq!(p.label(true), "⇧⌘P");
        let m = Cmd::MoveLineUp.shortcut().expect("move has one");
        assert_eq!(m.label(false), "Alt+↑");
        assert_eq!(m.label(true), "⌥↑");
    }

    #[test]
    fn labels_are_not_empty() {
        for c in Cmd::ALL {
            assert!(!c.label().is_empty() && !c.id().is_empty());
        }
    }
}
