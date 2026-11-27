# Código app icon

**Figure:** Alan Turing (1912–1954), who in 1936 defined what a program can compute with the machine
that bears his name, and so gave programming its theory.

**Style:** a black-and-white portrait cut out over a full-bleed colour field, head and shoulders,
the bust running off the bottom edge, in the Nodalia icon family (one historical figure per app,
one colour per app). The tile is Código teal, `#12B094` at the top to `#0B9A80` at the bottom.

**Provenance:** the portrait is the passport photograph of Alan Turing at 16 (1928–1929, possibly by
Arthur Reginald Chaffin; Turing Digital Archive, King's College, Cambridge), in the public domain, on
[Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Alan_Turing_Aged_16.jpg). The figure was
cut out with nodalia-design's `tools/app-portrait`.

## Files

| File | What |
|---|---|
| `codigo-portrait-1024.png` | the artwork: full-bleed, 1024 px |
| `codigo.svg` | master: the artwork in a 512 rounded tile (radius 112) |
| `codigo-linux.svg` | Colloid's application grid: a 224 tile at 16 on a 256 canvas, soft shadow |
| `codigo-1024.png` | the master on Apple's icon grid (824 px tile, transparent margin); `packaging/macos/package.sh` builds the `.icns` from it |
| `codigo.ico` | Windows: the sizes 16–256 of the Linux set packed into one icon (PNG entries); `apps/codigo/build.rs` embeds it in `codigo.exe` and the MSI uses it for Apps & features |
| `hicolor/<size>x<size>/apps/dev.danielmiguelt.Codigo.png`, `hicolor/scalable/…svg` | Linux icon theme (the 256 px one is also the window icon and the logo in the app) |

## Regenerate

```sh
# the artwork, with nodalia-design's tools/app-portrait/portrait.py (see its README):
python3 portrait.py turing.jpg codigo-portrait-1024.png --box 30,10,670,650 --colors '#12B094' '#0B9A80'
# every file above:
python3 icon-set.py codigo-portrait-1024.png assets/icons --slug codigo --app-id dev.danielmiguelt.Codigo --comment '…'
```

`turing.jpg` is the 675 × 919 px original from Commons.
