# Architecture

```text
apps/codigo            egui app: window, editor widget, explorer, overlays, menus, translations
crates/codigo-engine   documents (buffer + syntax + file), commands, settings, session
crates/codigo-syntax   languages and tree-sitter highlighting
crates/codigo-workspace  folder walk, tree, fuzzy matching, project search, git status
crates/codigo-text     rope buffer, selections, edits, history, motions, search, encodings
```

Dependencies point down only. The engine knows nothing about egui, so another front end (a terminal
client, a headless tool) can drive the same documents and commands.

## Text

A `Buffer` wraps a rope and addresses text in chars. Selections are ranges with a head; several can
exist at once and every edit applies to all of them in one history step. Search runs on the rope
without copying it.

## Drawing the editor

The editor is a fixed-cell grid (`layout.rs`): only the rows in the viewport are laid out and
painted, so cost follows the screen, not the file. Word wrap needs every line's height up front, so
it is switched off past `WRAP_LINE_LIMIT` lines. Highlight spans are requested only for visible
lines and cached per document version.

## Commands

`Cmd` is the single list of actions: id, label, menu, default shortcut. The palette, the menus, the
global menu and key handling all dispatch the same values, which is also what a future scripting or
agent interface will use.

## Settings and session

`Settings` and `SessionState` are JSON files in the config directory (`CODIGO_CONFIG_DIR` overrides
it). Every field has a default and a sanitising pass, so a file from another version, or a broken
one, never stops the app from starting.
