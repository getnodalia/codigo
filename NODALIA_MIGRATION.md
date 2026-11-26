# Nodalia migration: Código

Category: new app, built for Nodalia from the start (no upstream).
Status: first version on a pull request, 2026-11-26.

## Current state

Rust workspace: `codigo-text`, `codigo-syntax`, `codigo-workspace`, `codigo-engine` and the egui app
`codigo-app` (binary `codigo`), with `nodalia-theme` and `nodalia-update`. CI calls Foundation's
`rust-gate`, `git-sources` and `agents-check` workflows, pinned by commit, plus the release-notes
check, and the release workflows (`release-auto`, `release-linux`, `release-macos` with a DMG
window in Código's colour, `release-windows`) through Foundation's `release-tag` and
`release-publish`.

## Desired state

Creator suite look with the shared top bar, Home and dialogs; a Windows installer (MSI) beside the
portable zip; website page getnodalia.com/codigo and a Nodalia Launcher catalog entry.

## Inherited dependencies still pending

None.

## Reusable Nodalia components

- `nodalia-theme` (egui, `creator` feature): `Accent::Codigo` (`#0B9A80`), islands, buttons, fields,
  window controls, icons and the About page. The accent lands in nodalia-design #85.
- `nodalia-update`: the in-app update check.
- `tools/app-portrait` in nodalia-design made the icon (Ada Lovelace).

## NDL work pending

- The editor, tabs, explorer rows and status bar are drawn in Código; if another app needs them,
  they move to nodalia-design.
- The Windows MSI, the website page and the Launcher entry.

## Missing tests

- The views are checked by hand with screenshots; a headless render test of the main screens would
  catch layout regressions.
