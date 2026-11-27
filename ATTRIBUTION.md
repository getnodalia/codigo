# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/codigo-portrait-1024.png`, `assets/icons/codigo*.svg`, `assets/icons/codigo-1024.png` | App icon artwork: Alan Turing (1912–1954), who defined what a program can compute, cut out in black and white on Código's green | Nodalia, from his passport photograph at 16 (1928–1929, possibly by Arthur Reginald Chaffin; Turing Digital Archive, King's College, Cambridge) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Alan_Turing_Aged_16.jpg); made with nodalia-design `tools/app-portrait` | Portrait: public domain. Artwork: MIT OR Apache-2.0 |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Codigo.*`, `assets/icons/codigo.ico` | App icon at each size, on the Colloid application grid | Nodalia | made from the artwork above with nodalia-design `tools/app-portrait/icon-set.py`; the `.ico` packs the 16–256 px PNGs | MIT OR Apache-2.0 |
| `assets/brand/github-mark.png` | The GitHub mark (the Invertocat silhouette), white on transparent, shown tinted beside the GitHub panel, sign-in and menus | Rendered by Nodalia from the Simple Icons path data | [Simple Icons](https://github.com/simple-icons/simple-icons) (`icons/github.svg`) | Simple Icons: CC0-1.0. The mark is a trademark of GitHub, Inc., used only to name the service |
| `assets/brand/gitlab-mark.png` | The GitLab mark (the tanuki silhouette), white on transparent, shown tinted beside the GitLab panel, sign-in and menus | Rendered by Nodalia from the Simple Icons path data | [Simple Icons](https://github.com/simple-icons/simple-icons) (`icons/gitlab.svg`) | Simple Icons: CC0-1.0. The mark is a trademark of GitLab B.V., used only to name the service |

Interface icons, colours and the window controls are drawn by `nodalia-theme`, from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT). Interface text uses egui's
default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font, under their own open licences,
inside the `epaint` crate). Characters those fonts lack are drawn with fonts already installed on
the computer, read at run time and never shipped. Syntax grammars come from the `tree-sitter-*`
crates (MIT).
