// SPDX-License-Identifier: MIT OR Apache-2.0
//! The menus Código shows in the desktop's top bar (the Linux global menu), as (label, id)
//! pairs generated from the command table. `-` is a separator. Every id is a command id, except
//! [`UPDATES`].

use codigo_engine::Cmd;

use crate::i18n::tr;

pub type Entry = (&'static str, &'static str);

/// "Check for updates…", the one menu item that is not a command.
pub const UPDATES: &str = "app.updates";

const SEP: Entry = ("-", "");

fn items(cmds: &[Option<Cmd>]) -> Vec<Entry> {
    cmds.iter()
        .map(|c| match c {
            Some(c) => (tr(c.label()), c.id()),
            None => SEP,
        })
        .collect()
}

pub fn tree() -> Vec<(&'static str, Vec<Entry>)> {
    use Cmd::*;
    let some = Some;
    let mut app = items(&[
        some(About),
        None,
        some(Preferences),
        some(KeyboardShortcuts),
    ]);
    app.push((tr("Check for updates…"), UPDATES));
    app.push(SEP);
    app.push((tr(Quit.label()), Quit.id()));
    vec![
        ("Código", app),
        (
            tr("File"),
            items(&[
                some(NewFile),
                some(OpenFile),
                some(OpenFolder),
                None,
                some(Save),
                some(SaveAs),
                some(SaveAll),
                None,
                some(CloseTab),
                some(CloseAllTabs),
                some(ReopenClosedTab),
            ]),
        ),
        (
            tr("Edit"),
            items(&[
                some(Undo),
                some(Redo),
                None,
                some(Cut),
                some(Copy),
                some(Paste),
                None,
                some(Find),
                some(Replace),
                some(FindInProject),
                None,
                some(ToggleLineComment),
                some(Indent),
                some(Outdent),
                None,
                some(MoveLineUp),
                some(MoveLineDown),
                some(DuplicateLineUp),
                some(DuplicateLineDown),
                some(DeleteLine),
                some(JoinLines),
                None,
                some(TriggerCompletion),
            ]),
        ),
        (
            tr("Selection"),
            items(&[
                some(SelectAll),
                some(SelectLine),
                None,
                some(SelectNextOccurrence),
                some(SelectAllOccurrences),
                None,
                some(AddCursorAbove),
                some(AddCursorBelow),
            ]),
        ),
        (
            tr("View"),
            items(&[
                some(CommandPalette),
                None,
                some(ToggleSidebar),
                some(ToggleWordWrap),
                some(ToggleWhitespace),
                None,
                some(ZoomIn),
                some(ZoomOut),
                some(ZoomReset),
            ]),
        ),
        (
            tr("Go"),
            items(&[
                some(QuickOpen),
                some(GoToLine),
                some(GoToDefinition),
                some(ShowHover),
                None,
                some(NextTab),
                some(PreviousTab),
            ]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_id_is_a_command_or_the_update_check() {
        for (_, entries) in tree() {
            for (label, id) in entries {
                if label != "-" {
                    assert!(id == UPDATES || Cmd::from_id(id).is_some(), "{id}");
                }
            }
        }
    }

    #[test]
    fn no_command_is_listed_twice() {
        let mut seen = std::collections::HashSet::new();
        for (_, entries) in tree() {
            for (label, id) in entries {
                if label != "-" {
                    assert!(seen.insert(id), "{id} appears twice");
                }
            }
        }
    }
}
