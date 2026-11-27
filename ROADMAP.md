# Roadmap

Código aims to be an editor you can work in all day, in any language, with an AI beside you. This
file lists what exists and what does not. Nothing under "Next" is available yet.

## Exists (0.6)

Editor core (multi-cursor, find and replace, word wrap, undo, encodings), tree-sitter highlighting
for 15 languages, explorer with git status, quick open, project search, command palette, session
restore, language server diagnostics, hover, completion, go to definition, references, rename and format, snippets, a minimap and a split editor, the Engineering suite look (IBM Plex type and shared panels), a drawn symbol per file type in the explorer and tabs, a terminal and run console with Run, Stop and Terminal buttons in the top bar, a Problems panel, a debugger (breakpoints, stepping, stack, variables and evaluation through `lldb-dap`, GDB, `dlv` and `debugpy`), Git in the window (changes, diffs, stage, commit, branches, fetch, pull, push), GitHub and GitLab in the window (clone, pull requests and merge requests, issues and reviews, with the services' own marks), a large Preferences window with colour themes and fonts, files that open at once, Run and Debug in any project, AI assistants (chat, explain, fix, refactor, document and tests through Anthropic, OpenAI, Ollama or a compatible server, keys in the keyring), stash, merge and conflict resolution, staging by hunk, blame at the end of the line, creating a pull request and comments on a diff line, a Windows installer (MSI), code quality and security analysis, a New Nodalia app template, Spanish and English, Nodalia Design themes, Linux global menu, update check.

## Next, in this order

1. **Language servers (LSP).** Diagnostics, hover, completion, go to definition, references, rename
   and formatting, with servers started per language from the user's PATH or settings. The
   `codigo-lsp` crate (protocol client, off-thread transport, server registry) and the engine's
   `Hub` run: diagnostics show as underlines, tooltips and status bar counts, and hover, completion,
   go to definition, references, rename and formatting work. Still to do: code actions, signature
   help, semantic tokens and inlay hints.
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
5. **AI assistants** are in (chat about the file or selection, explain, fix, refactor, document and
   write tests, applied as an undoable edit, with the provider and key chosen by the user and
   the key kept in the system's keyring). Still to do: inline completions as you type, edits
   across several files, review of a change for quality and security, and trying the real
   Anthropic and OpenAI services and the keyring.
6. **Git in the editor** is in (changes, diffs, stage by hunk, commit, branches, push and pull,
   stash, merge and conflict resolution, blame at the end of the line, and GitHub pull requests,
   issues, cloning, creating a pull request and line comments on a diff); still to do: trying the
   GitHub writes against a real repository, a blame gutter, and a three-way merge editor.
7. **Debugging** is in (see above); still to do: conditional breakpoints, watches that persist,
   breakpoints that follow edits, tried-and-tested Go and Python sessions, and launch
   configurations. Also here: a memory budget for very large files. Snippets, a split editor and a
   minimap exist; still to do for them: more than two panes, drag a tab to split, and snippets from
   extensions.
8. **Distribution**: the packages exist (AppImage, .deb, .rpm, tarball, a macOS DMG with its own
   install window, and for Windows a per-user MSI and a portable zip); still to do: the website
   page and a Nodalia Launcher entry.

## Known gaps

- Highlighting covers the 15 languages above; other grammars are added as they are needed.
- A 45 MB, million-line file takes about 1.3 GB of memory with highlighting on and parses on
  load; large files need lighter parsing and a memory budget.
- The editor draws a fixed-width cell grid; proportional fonts are not supported.
- No right-to-left layout yet.
