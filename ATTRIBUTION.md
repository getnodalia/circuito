# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|

| `apps/circuito/src/icon.rs` | Placeholder app icon (a resistor zigzag between two terminals on the accent colour), drawn in code | Nodalia | original work | `MIT OR Apache-2.0` |
| `assets/app-icon/circuito.png`, `assets/app-icon/circuito.ico`, `assets/app-icon/circuito.icns`, `assets/app-icon/hicolor/*/apps/dev.danielmiguelt.Circuito.png` | The placeholder icon above in the sizes and formats the installers need, rendered by `packaging/icons.sh` | Nodalia | original work (rendered from `icon.rs`) | `MIT OR Apache-2.0` |
| `packaging/macos/dmg/background.svg`, `packaging/macos/dmg/background.tiff` | The Finder window of the macOS DMG: an RC low-pass schematic and its step response on Circuito's accent, laid out like the other Nodalia DMGs. The lettering is outlined from Sora | Nodalia | original work, drawn by `packaging/macos/dmg/draw_background.py`; Sora from [nodalia-design](https://github.com/getnodalia/nodalia-design) | `MIT OR Apache-2.0`; Sora `OFL-1.1` |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder's view settings for that window, written by `packaging/macos/dmg/generate.py` | Nodalia | original work | `MIT OR Apache-2.0` |

No other asset files are committed. Interface icons, colours and the window controls are drawn by
`nodalia-theme`, from [nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT).
Interface text uses egui's default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font,
under their own open licences, inside the `epaint` crate).
