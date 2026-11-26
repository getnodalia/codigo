# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/codigo-portrait-1024.png`, `assets/icons/codigo*.svg`, `assets/icons/codigo-1024.png` | App icon artwork: Grace Hopper (1906–1992), who wrote the first compiler, cut out in black and white on Código's green | Nodalia, from the official U.S. Navy portrait of 1984 by James S. Davis | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Commodore_Grace_M._Hopper,_USN_(covered).jpg); made with nodalia-design `tools/app-portrait` | Portrait: public domain (a work of the U.S. federal government). Artwork: MIT OR Apache-2.0 |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Codigo.*`, `assets/icons/codigo.ico` | App icon at each size, on the Colloid application grid | Nodalia | made from the artwork above with nodalia-design `tools/app-portrait/icon-set.py`; the `.ico` packs the 16–256 px PNGs | MIT OR Apache-2.0 |

Interface icons, colours and the window controls are drawn by `nodalia-theme`, from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT). Interface text uses egui's
default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font, under their own open licences,
inside the `epaint` crate). Characters those fonts lack are drawn with fonts already installed on
the computer, read at run time and never shipped. Syntax grammars come from the `tree-sitter-*`
crates (MIT).
