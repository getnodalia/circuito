# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|

| `assets/icons/circuito-portrait-1024.png`, `assets/icons/circuito*.svg`, `assets/icons/circuito-1024.png` | App icon artwork: Gustav Kirchhoff (1824–1887), whose circuit laws underlie every simulator here, cut out in black and white on Circuito's rose copper | Nodalia, from a 19th-century portrait photograph (author unknown) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Gustav_Robert_Kirchhoff.jpg), from the Smithsonian Libraries; made with nodalia-design `tools/app-portrait` | Portrait: public domain. Artwork: `MIT OR Apache-2.0` |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Circuito.*` | App icon at each size, on the Colloid application grid | Nodalia | made from the artwork above with nodalia-design `tools/app-portrait/icon-set.py` | `MIT OR Apache-2.0` |
| `assets/app-icon/circuito.ico`, `assets/app-icon/circuito.icns` | The same icon as a Windows icon and a macOS icon, rendered by `packaging/icons.sh` | Nodalia | made from `assets/icons/circuito-1024.png` | `MIT OR Apache-2.0` |
| `packaging/macos/dmg/background.svg`, `packaging/macos/dmg/background.tiff` | The Finder window of the macOS DMG: an RC low-pass schematic and its step response on Circuito's accent, laid out like the other Nodalia DMGs. The lettering is outlined from Sora | Nodalia | original work, drawn by `packaging/macos/dmg/draw_background.py`; Sora from [nodalia-design](https://github.com/getnodalia/nodalia-design) | `MIT OR Apache-2.0`; Sora `OFL-1.1` |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder's view settings for that window, written by `packaging/macos/dmg/generate.py` | Nodalia | original work | `MIT OR Apache-2.0` |

No other asset files are committed. Interface icons, colours and the window controls are drawn by
`nodalia-theme`, from [nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT).
Interface text uses egui's default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font,
under their own open licences, inside the `epaint` crate).
