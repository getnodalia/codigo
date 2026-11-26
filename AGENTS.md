# Código: instructions for agents

Engineering rules for every Nodalia repository: [getnodalia/foundation](https://github.com/getnodalia/foundation)
([engineering](https://github.com/getnodalia/foundation/blob/main/standards/engineering.md),
[CI](https://github.com/getnodalia/foundation/blob/main/standards/ci.md),
[security](https://github.com/getnodalia/foundation/blob/main/standards/security.md),
[dependencies and licences](https://github.com/getnodalia/foundation/blob/main/standards/dependencies.md),
[release](https://github.com/getnodalia/foundation/blob/main/standards/release.md),
[NDL](https://github.com/getnodalia/foundation/blob/main/standards/ndl.md)).

## What this is

Código is Nodalia's code editor and IDE: a fast, native editor for any language, with language
servers, an integrated terminal and AI assistants as the roadmap's next steps. It is written in
**Rust only**: the interface is egui/eframe with `nodalia-theme`, and there is no Electron,
Chromium or web view. The product name is **Código** in prose and in the interface. Machine names
are `codigo` (crates, binary, files). The app ID is `dev.danielmiguelt.Codigo`, the accent is
`#0B9A80` (`Accent::Codigo`) and the licence is `MIT OR Apache-2.0`. Código is its own app: never
describe it as a fork or a port. Read `README.md` for the user-facing picture and `ROADMAP.md` for
what does not exist yet.

## Architecture

| Path | Responsibility |
|---|---|
| `crates/codigo-text` | The buffer (ropey, char offsets), selections and multi-cursor, edits, undo history, motions, search, encodings. No I/O beyond decoding bytes |
| `crates/codigo-syntax` | Languages, tree-sitter parsing and highlight scopes |
| `crates/codigo-lsp` | The language server client: JSON-RPC framing, UTF-16 positions, the off-thread `Client`, the server registry and spawning. No UI, no engine types |
| `crates/codigo-workspace` | Folder walking with ignore rules, the tree, fuzzy matching, project search, git status |
| `crates/codigo-engine` | Documents (buffer + syntax + file), commands, settings, session. No UI |
| `apps/codigo` | The egui app: `app.rs` (state and frame loop), `ui.rs` (window), `editor.rs`, `explorer.rs`, `overlays.rs` (palette, quick open, preferences, About…), `menu.rs`, `linux_menu.rs`, `spanish.rs`, `i18n.rs` |

Dependencies point down: text ← syntax/workspace ← engine ← app. Nothing below `apps/codigo` may
use egui, eframe or winit. Anything a future terminal or headless client would also need belongs in
the engine, not in the app.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked   # the CI gate
cargo run -p codigo-app -- path/to/folder-or-file
CODIGO_CONFIG_DIR=/tmp/cfg CODIGO_CACHE_DIR=/tmp/cch cargo run -p codigo-app   # a clean profile
```

To look at the UI with no display, run it under `Xvfb` and drive it with `xdotool`; Linux needs
`libxkbcommon-x11-0` and `libegl1` at run time. Focus the window (`xdotool windowfocus`) before
sending keys.

## Rules

- **Never crash.** Non-test code has no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`
  and no `unsafe` (workspace lints, `-D warnings`). Files, encodings, grammars and every future server
  reply are hostile: return errors and show them, cap sizes, use `get()` and char boundaries.
- **Never block the UI thread** on disk, git, search, parsing of big files or the network.
- **Everything is a command.** User-visible behaviour is a `Cmd` in the engine, reachable from the
  palette, the menus and the shortcuts. Programmatic calls never open dialogs.
- **Every user-visible string** goes through `i18n::tr`/`trf` with its Spanish in the sorted `SPANISH`
  table (tests enforce both). Colours come from `nodalia-theme` tokens, never from a literal.
- **Text is Unicode.** Offsets are chars, widths come from `unicode-width`, and save/reload keeps
  the file's encoding and line endings byte for byte where possible.
- **Secrets** (API keys for AI providers, tokens) never go to disk in plain text, logs or errors.
- **Assets** get a row in `ATTRIBUTION.md` in the same change. New source files carry
  `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- Planned features are never described as existing: README, ROADMAP and release notes say only what runs.
- Commits: conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`).

## NDL

`nodalia-theme` comes from getnodalia/nodalia-design, pinned by `rev` with the `egui` and `creator`
features. It supplies the palette, islands, buttons, fields, window controls, icons and the About
page. Never copy NDL into this repository; a missing component goes to nodalia-design first. NDL
window controls are drawn on Linux only (unless the person picks the system's own in Preferences);
macOS and Windows keep native controls. A hamburger menu in the header holds Preferences, Keyboard
shortcuts and About. On Linux the app menu is exported over D-Bus to the shell's global menu. About
has no Contributors or Models tabs. What is still missing is in `NODALIA_MIGRATION.md`.

## CI workflows

- Workflows call getnodalia/foundation's reusable workflows, pinned by commit.
- Pin every action to a full commit SHA, give each job the least permissions it needs, and use only
  standard GitHub-hosted runners (zero cost; macOS jobs only for releases).
- Keep CI changes in their own pull request unless a change cannot work without them.

## Release

A pull request that sets `[workspace.package] version` to a release version and adds
`docs/release-notes/vX.Y.Z.md` (format in that folder's README, checked by
`scripts/check-release-notes.sh`) releases it when it merges. Releases that add features bump the
minor, fix-only releases the patch. `main` carries a `-dev` version between releases. Nobody pushes
a tag by hand. The release workflows (Linux, macOS DMG with its own window, Windows) are not in the
repository yet; see `NODALIA_MIGRATION.md`.

## Pull requests

A change is ready when the gate passes, new behaviour has a test that fails without it, UI changes
come with a screenshot taken from the running app, and docs that describe the behaviour are updated.
`main` changes only through pull requests.
