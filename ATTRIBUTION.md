# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/codigo-portrait-1024.png` | App icon artwork: Ada Lovelace (1815–1852), who wrote the first published program, cut out in black and white on Código's green | Nodalia, from the 1840 portrait of Ada Lovelace by Alfred Edward Chalon | [Wikimedia Commons](https://commons.wikimedia.org/wiki/Category:Ada_Lovelace); made with nodalia-design `tools/app-portrait` | Portrait: public domain (the painter died in 1860). Artwork: MIT OR Apache-2.0 |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Codigo.png` | App icon at each size (rounded tile of the artwork above) | Nodalia | made from the artwork above | MIT OR Apache-2.0 |

Interface icons, colours and the window controls are drawn by `nodalia-theme`, from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT). Interface text uses egui's
default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font, under their own open licences,
inside the `epaint` crate). Characters those fonts lack are drawn with fonts already installed on
the computer, read at run time and never shipped. Syntax grammars come from the `tree-sitter-*`
crates (MIT).
