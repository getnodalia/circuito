# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side. `package.sh` copies these files into the image; nothing here is
generated at build time, so the DMG builds with `hdiutil create -srcfolder` (no Finder scripting on
CI, nothing extra installed in the job).

| File | What |
|---|---|
| `draw_background.py` | Writes `background.svg`. The layout and the lettering are [nodalia-design](https://github.com/getnodalia/nodalia-design)'s (`tools/dmg-background/draw_background.py`, imported from a checkout named by `NODALIA_DESIGN_DIR`); this file adds Circuito to its table: two Nodalia islands on the window colour, the left one in Circuito's colour (`#C99A06`) with an RC low-pass drawn as a schematic and its step response, the right one white under the icons, the arrow a resistor on a wire. Delete it when nodalia-design has a Circuito entry of its own. |
| `background.svg` | The drawing. Its text (Sora, OFL-1.1) is outlined, so rendering it needs no fonts. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (16-colour palette, Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, `Circuito.app` at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store` in the image. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |

## The volume name has no version

The mounted volume is called `Circuito`, not `Circuito <version>`. The layout points at the
background through an alias that includes the volume name, and Finder resolves it by that name: with
a versioned name the window keeps its size and icon positions but shows no background. The DMG file
name (`circuito-<version>-macos-<arch>.dmg`) still carries the version. `package.sh` stops the build
if `dmg-layout.DS_Store` was laid out for another volume or app name.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the area under both icons stays light (the white island).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and of
  the label strip under it.
- Flat inks, no gradients or glows.

## Regenerate

```sh
pip install fonttools 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
NODALIA_DESIGN_DIR=/path/to/nodalia-design python3 packaging/macos/dmg/draw_background.py
python3 packaging/macos/dmg/generate.py   # needs resvg on PATH (cargo install resvg --locked)
```

Both outputs are byte-for-byte reproducible and carry nothing from the machine that ran them:
`dmg-layout.DS_Store` is written from scratch, its background alias holding only the volume name and
`/.background/background.tiff`. The window is 660 × 432 with Finder's 32 pt title bar; the content
area is 660 × 400. The layout files were not opened in Finder on a Mac: the DMG is built unsigned
(ad hoc) in CI and the window has to be looked at on the first release.
