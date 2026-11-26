# Roadmap

Código aims to be an editor you can work in all day, in any language, with an AI beside you. This
file lists what exists and what does not. Nothing under "Next" is available yet.

## Exists (0.1, in development)

Editor core (multi-cursor, find and replace, word wrap, undo, encodings), tree-sitter highlighting
for 15 languages, explorer with git status, quick open, project search, command palette, session
restore, Spanish and English, Nodalia Design themes, Linux global menu, update check.

## Next, in this order

1. **Language servers (LSP).** Diagnostics, hover, completion, go to definition, references, rename
   and formatting, with servers started per language from the user's PATH or settings. The
   `codigo-lsp` crate (protocol client, off-thread transport, server registry) exists; it is not
   wired into the editor yet, so nothing in the interface uses it.
2. **Integrated terminal.** A pty-backed terminal panel (`codigo-term`) with tabs and the project
   folder as its working directory.
3. **AI assistants.** Chat, inline edits and completions through the shared `nodalia-assistant`
   crate, with the provider chosen by the user and keys kept in the system keyring. Nothing leaves
   the computer without being asked.
4. **Git in the editor.** Changes view, diff gutter, stage, commit and branch switching.
5. **Debugging** through the Debug Adapter Protocol, tasks, snippets, split editors and a
   minimap.
6. **Release packages**: AppImage, .deb and .rpm, a macOS DMG with its own install window, a Windows
   installer, the website page and a Nodalia Launcher entry.

## Known gaps

- Highlighting covers the 15 languages above; other grammars are added as they are needed.
- A 45 MB, million-line file takes about 1.3 GB of memory with highlighting on and parses on
  load; large files need lighter parsing and a memory budget.
- The editor draws a fixed-width cell grid; proportional fonts are not supported.
- No right-to-left layout yet.
