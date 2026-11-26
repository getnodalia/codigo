# Código app icon

**Figure:** Grace Hopper (1906–1992), who wrote the first compiler and pushed programming towards
languages people can read, and who made "debugging" a word every programmer knows.

**Style:** a black-and-white portrait cut out over a full-bleed colour field, head and shoulders,
the bust running off the bottom edge, in the Nodalia icon family (one historical figure per app,
one colour per app). The tile is Código teal, `#12B094` at the top to `#0B9A80` at the bottom.

**Provenance:** the portrait is the official U.S. Navy photograph "Commodore Grace M. Hopper, USN
(covered)" (1984, by James S. Davis), a work of the U.S. federal government and so in the public
domain, on [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Commodore_Grace_M._Hopper,_USN_(covered).jpg).
The figure was cut out with nodalia-design's `tools/app-portrait`.

## Files

| File | What |
|---|---|
| `codigo-portrait-1024.png` | the artwork: full-bleed, 1024 px |
| `codigo.svg` | master: the artwork in a 512 rounded tile (radius 112) |
| `codigo-linux.svg` | Colloid's application grid: a 224 tile at 16 on a 256 canvas, soft shadow |
| `codigo-1024.png` | the master on Apple's icon grid (824 px tile, transparent margin); `packaging/macos/package.sh` builds the `.icns` from it |
| `hicolor/<size>x<size>/apps/dev.danielmiguelt.Codigo.png`, `hicolor/scalable/…svg` | Linux icon theme (the 256 px one is also the window icon and the logo in the app) |

## Regenerate

```sh
# the artwork, with nodalia-design's tools/app-portrait/portrait.py (see its README):
python3 portrait.py hopper.jpg codigo-portrait-1024.png --box 672,228,1872,1428 --colors '#12B094' '#0B9A80'
# every file above:
python3 icon-set.py codigo-portrait-1024.png assets/icons --slug codigo --app-id dev.danielmiguelt.Codigo --comment '…'
```

`hopper.jpg` is the 1920 px version from Commons.
