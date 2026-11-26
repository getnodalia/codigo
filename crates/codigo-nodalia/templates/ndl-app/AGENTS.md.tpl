# {{name}}: instructions for agents

Engineering rules for every Nodalia repository: [getnodalia/foundation](https://github.com/getnodalia/foundation)
([engineering](https://github.com/getnodalia/foundation/blob/main/standards/engineering.md),
[CI](https://github.com/getnodalia/foundation/blob/main/standards/ci.md),
[security](https://github.com/getnodalia/foundation/blob/main/standards/security.md),
[dependencies and licences](https://github.com/getnodalia/foundation/blob/main/standards/dependencies.md),
[release](https://github.com/getnodalia/foundation/blob/main/standards/release.md),
[NDL](https://github.com/getnodalia/foundation/blob/main/standards/ndl.md)).

## What this is

{{name}} is a Nodalia app written in **Rust only**: the interface is egui/eframe with
`nodalia-theme`, and there is no Electron, Chromium or web view. The product name is **{{name}}**
in prose and in the interface. Machine names are `{{slug}}`. The app ID is `{{app_id}}` and the
licence is `MIT OR Apache-2.0`.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked   # the CI gate
cargo run
```

## Rules

- **Never crash.** Non-test code has no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`
  and no `unsafe` (lints, `-D warnings`). Files and every other input are hostile: return errors
  and show them.
- **Never block the UI thread** on disk, the network or anything slow.
- **Every user-visible string** goes through `i18n::tr`/`trf` with its Spanish in the sorted
  `SPANISH` table (tests enforce both). Colours come from `nodalia-theme` tokens, never from a literal.
- **Assets** get a row in `ATTRIBUTION.md` in the same change. New source files carry
  `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- Planned features are never described as existing.
- Commits: conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`).

## NDL

`nodalia-theme` comes from getnodalia/nodalia-design, pinned by `rev`. Never copy NDL into this
repository; a missing component goes to nodalia-design first. NDL window controls are drawn on
Linux only; macOS and Windows keep native controls.

## CI workflows

- Workflows call getnodalia/foundation's reusable workflows, pinned by commit.
- Pin every action to a full commit SHA, give each job the least permissions it needs, and use only
  standard GitHub-hosted runners (zero cost).
- Keep CI changes in their own pull request unless a change cannot work without them.
