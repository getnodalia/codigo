# Código

Código is Nodalia's code editor: a fast, native editor for any language, drawn in the Nodalia
Design Language and written entirely in Rust. It does not run on Electron, Chromium or a web view.

> Código is at an early stage (0.1, in development). This page describes only what runs today;
> what is planned is in [ROADMAP.md](ROADMAP.md).

## What it does today

- **A text editor built for speed.** Only the rows on screen are laid out, so a file of a million
  lines (45 MB) opens and scrolls without trouble. Multiple cursors and selections, word wrap, find and replace with a
  match count, line operations (move, duplicate, delete, join, comment, indent), unlimited undo and
  Unicode that behaves: grapheme-aware movement, wide characters, and the file's encoding and line
  endings kept when you save.
- **Syntax highlighting for 15 languages** through tree-sitter: Rust, Python, JavaScript,
  TypeScript, JSON, TOML, Markdown, Bash, C, C++, Go, Java, HTML, CSS and YAML. Other files open as
  plain text, and the status bar lets you pick a language by hand.
- **Errors as you type.** With a language server installed (rust-analyzer, pyright, gopls,
  clangd, typescript-language-server and others), Código underlines what it reports, shows the
  message under the pointer and counts errors and warnings in the status bar. It never installs a
  server for you, and the setting can be turned off in Preferences. Hover (`Ctrl+Shift+Space`),
  completion (`Ctrl+Space`, and as you type) and go to definition (`F12`) work too; references,
  rename and formatting are not there yet.
- **A terminal and a run console.** `Ctrl+\`` opens a real terminal under the editor, with tabs,
  colours, history and the project folder as its working directory. `F5` runs the project
  (`cargo run`, `npm start`, `go run .`, `python main.py` and more, chosen from what the folder
  contains) in a tab of its own, and Build, Test and Check are in the Run menu. A
  `path:line:column` in the output is a link: click it to open that place.
- **Files you can tell apart.** Every file type has its own drawn symbol in the explorer and in
  the tabs, so you find the Rust, the Python or the Dockerfile at a glance.
- **Projects.** Open a folder to get a file explorer that respects `.gitignore` and shows git status
  in colour, quick open by fuzzy file name (`Ctrl+P`), search across the project (`Ctrl+Shift+F`)
  and a command palette (`Ctrl+Shift+P`) that reaches every command.
- **Pick up where you left off.** Tabs, the folder, the sidebar and recent folders come back at the
  next launch.
- **Nodalia Design.** Light, dark and high-contrast themes, round window controls on Linux (native
  ones on macOS and Windows), a global menu over D-Bus on Linux, a reduced-motion option and an
  in-app update check.
- **Spanish and English**, following your system language or your choice in Preferences.

## Build

```bash
cargo run -p codigo-app -- path/to/folder-or-file
```

Linux needs the usual development packages for windowing (`libxkbcommon`, `libwayland`, `libx11`,
`libgl`, `libfontconfig`, `libdbus`). The gate that CI runs:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
```

## Layout

`crates/codigo-text` (buffer, selections, edits, history, search), `crates/codigo-syntax`
(languages and highlighting), `crates/codigo-lsp` (language server client), `crates/codigo-term` (terminal), `crates/codigo-workspace` (folders, fuzzy matching, project search,
git), `crates/codigo-engine` (documents, commands, settings, session) and `apps/codigo` (the egui
app). See [AGENTS.md](AGENTS.md) for the rules the code follows.

## Licence

`MIT OR Apache-2.0` ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). Assets and their
licences are listed in [ATTRIBUTION.md](ATTRIBUTION.md).
