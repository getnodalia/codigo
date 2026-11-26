# Roadmap

Código aims to be an editor you can work in all day, in any language, with an AI beside you. This
file lists what exists and what does not. Nothing under "Next" is available yet.

## Exists (0.1)

Editor core (multi-cursor, find and replace, word wrap, undo, encodings), tree-sitter highlighting
for 15 languages, explorer with git status, quick open, project search, command palette, session
restore, language server diagnostics, hover, completion and go to definition, a drawn symbol per file type in the explorer and tabs, a terminal and run console, code quality and security analysis, a New Nodalia app template, Spanish and English, Nodalia Design themes, Linux global menu, update check.

## Next, in this order

1. **Language servers (LSP).** Diagnostics, hover, completion, go to definition, references, rename
   and formatting, with servers started per language from the user's PATH or settings. The
   `codigo-lsp` crate (protocol client, off-thread transport, server registry) and the engine's
   `Hub` run: diagnostics show as underlines, tooltips and status bar counts, and hover, completion
   and go to definition work. References, rename and formatting are still to do.
2. **Run console and integrated terminal.** The pty-backed terminal panel (`codigo-term`) with
   tabs, and a run console for run, build, test and check with clickable errors, exist. Still to
   do: tasks defined per project, problem matchers for more output formats, split terminals,
   and keeping terminals across restarts.
3. **Code quality and security, built in.** The direct analysis exists: Código's own rules for
   secrets, unsafe patterns, Rust panics, long and complex functions and duplication, in a Quality
   panel with a gate. Still to do: the language servers' diagnostics and each language's own
   linters (clippy and others) in the same panel, dependency advisories, more languages' rules,
   findings in the editor margin, per-project rule settings, and fixes that apply from the editor.
4. **Tools for building Nodalia apps.** A project template (New Nodalia app) and the Nodalia
   engineering-rule checks in the Quality panel (colours from tokens, strings through `tr()`,
   Spanish table complete, licence headers) exist. Still to do: more templates (GTK, a server),
   a component and theme preview, helpers for tokens and icons, and checks for the remaining
   rules (no unwrap in a crate without the lint, wasm builds, assets in `ATTRIBUTION.md`).
5. **AI assistants.** Chat, inline edits and completions through the shared `nodalia-assistant`
   crate, with the provider chosen by the user and keys kept in the system keyring. Nothing leaves
   the computer without being asked. Review of a change for quality and security comes with it.
6. **Git in the editor.** Changes view, diff gutter, stage, commit and branch switching.
7. **Debugging** through the Debug Adapter Protocol, snippets, split editors and a minimap, with
   lighter parsing and a memory budget for very large files.
8. **Distribution**: the packages exist (AppImage, .deb, .rpm, tarball, a macOS DMG with its own
   install window and a portable Windows zip); still to do: a Windows installer, the website page
   and a Nodalia Launcher entry.

## Known gaps

- Highlighting covers the 15 languages above; other grammars are added as they are needed.
- A 45 MB, million-line file takes about 1.3 GB of memory with highlighting on and parses on
  load; large files need lighter parsing and a memory budget.
- The editor draws a fixed-width cell grid; proportional fonts are not supported.
- No right-to-left layout yet.
