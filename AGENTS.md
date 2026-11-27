# Circuito: instructions for agents

Engineering rules for every Nodalia repository: [getnodalia/foundation](https://github.com/getnodalia/foundation)
([engineering](https://github.com/getnodalia/foundation/blob/main/standards/engineering.md),
[CI](https://github.com/getnodalia/foundation/blob/main/standards/ci.md),
[security](https://github.com/getnodalia/foundation/blob/main/standards/security.md),
[dependencies and licences](https://github.com/getnodalia/foundation/blob/main/standards/dependencies.md),
[release](https://github.com/getnodalia/foundation/blob/main/standards/release.md),
[NDL](https://github.com/getnodalia/foundation/blob/main/standards/ndl.md)).

## What this is

Circuito is Nodalia's electronics, electrical and industrial-automation application: schematic capture, circuit simulation, PCB design and manufacturing outputs, written in Rust with engines of its own. It belongs to the Engineering suite with Forja, Torno, Vortia and Ábaco. Written in **Rust only**: the interface is egui/eframe with `nodalia-theme`, and there is no
Electron, Chromium or web view. The product name is **Circuito** in prose and in the interface; machine
names are `circuito`. App ID `dev.danielmiguelt.Circuito`, accent `#C06A4F`, licence `MIT OR Apache-2.0`. Circuito is its
own app: never describe it as a fork or a port. `README.md` says what runs today and `ROADMAP.md`
what does not exist yet; planned features are never described as existing.

## Architecture

| Path | Responsibility |
|---|---|
| `crates/circuito-core` | Schematic and netlist model: parts, pins, nets, ids, values with engineering suffixes (`4.7k`, `100n`) |
| `crates/circuito-spice` | SPICE netlist reader and writer (R, C, L, V, I, D, E, G, `.model`, `.op`, `.dc`, `.ac`, `.tran`) |
| `crates/circuito-sim` | The circuit simulator: modified nodal analysis, Newton–Raphson, DC, AC and transient analyses |
| `crates/circuito-erc` | Netlist extraction from a schematic and the electrical rules check |
| `crates/circuito-pcb` | Board model, design rules check, Gerber RS-274X/X2 and Excellon output |
| `crates/circuito-kicad` | KiCad files: the S-expression reader and the `.kicad_pcb` importer (experimental) |
| `apps/circuito-cli` | Headless `circuito` command: `sim`, `erc`, `netlist`, `drc`, `gerber`, `import-kicad` |
| `apps/circuito` | The egui app (`circuito-app`): a library of testable logic (documents, findings, plots, settings, i18n) and the windows in the binary |

Dependencies point down: engines never use egui, eframe or winit, and no engine blocks on I/O it
was not asked to do. Anything a headless client (the CLI, a script, another Nodalia app) also needs
belongs in an engine crate, not in the app. A feature of a kind not listed above gets its own
crate when a second consumer exists, and a module of an existing crate before that.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked   # the CI gate
cargo run -p circuito-cli -- sim examples/rc-lowpass.cir   # headless
cargo run -p circuito-app -- examples/rc-lowpass.cir         # the app
```

Linux needs `libxkbcommon-x11-0` and `libegl1` at run time to run the app. Tests need no display,
fonts or network.

## Rules

- **Never crash** (workspace clippy lints deny `unwrap`, `expect`, `panic`, `todo`,
  `unimplemented`, `unreachable`, and `unsafe_code` is forbidden). Files, expressions, netlists and
  every number from outside are hostile: return errors with a position and a message the person can
  act on, use `get()` and checked arithmetic, cap sizes, depth and iterations.
- **Numbers are checked, never trusted.** An engine reports a singular or ill-conditioned system, a
  non-finite result or a failure to converge as an error or a warning with the numbers that show
  it; it never returns a plausible-looking wrong answer. A result carries what produced it (engine
  version, method, tolerances, iterations).
- **Verified against references.** A numerical feature lands with tests against analytic solutions
  or published reference values, with the tolerance stated and justified. Nothing is called
  validated that was not run. Say what was not tested.
- **Clean room.** Learn from public papers, standards, documentation and open-source designs;
  never copy code whose licence is not compatible, and never copy proprietary code, libraries,
  symbols, footprints or assets. Third-party code that is reused keeps its notice in `NOTICE`.
- **No wrappers.** The engines are Rust and Nodalia's own. No feature shells out to another
  program to do its work.
- **Never block the UI thread** on solving, parsing large files or I/O.
- **Every user-visible string** goes through `i18n::tr`/`trf` with its Spanish in the sorted table
  (tests enforce both). Colours come from `nodalia-theme` tokens, never from a literal.
- **Assets** get a row in `ATTRIBUTION.md` in the same change. New source files carry
  `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- **Unsafe physical consequences.** Nothing here may drive real equipment (machines, relays,
  industrial buses) from a simulation by accident: connectors are opt-in, explicit and isolated.
  No output is called certified or fit for critical use without evidence.
- Commits: conventional commits (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`).

## NDL

`nodalia-theme` from getnodalia/nodalia-design, pinned by `rev` with the `egui` and `engineering`
features (and `creator` for the About page). It supplies the palette, islands, ribbon, panels,
fields, window controls, icons and About. Never copy NDL into this repository; a missing component
goes to nodalia-design through its own pull request first. NDL window controls are drawn on Linux
only (unless the person picks the system's own); macOS and Windows keep native controls. A hamburger
menu holds Preferences, Keyboard shortcuts and About; the light/dark choice lives only in
Preferences; on Linux the app menu is exported over D-Bus to the shell's global menu. About has no
Contributors or Models tabs. What is missing is in `NODALIA_MIGRATION.md`.

## Release

A pull request that sets `[workspace.package] version` to a release version (no `-dev`) and adds
`docs/release-notes/vX.Y.Z.md` (format in that folder's README, checked by
`scripts/check-release-notes.sh`) releases it when it merges. Releases that add features bump the
minor; fix-only releases the patch. `main` carries a `-dev` version between releases. Nobody pushes
a tag by hand. `release-auto.yml` tags the merge through Foundation's `release-tag`;
`release-linux.yml` (AppImage, .deb, .rpm, .tar.gz), `release-macos.yml` (DMG with its own window,
ad hoc signed) and `release-windows.yml` (MSI and portable zip) build and attach the packages with
the scripts in `packaging/`, named `circuito-<version>-<os>-<arch>` as the Nodalia Launcher expects;
Foundation's `release-publish` publishes. The app icon is the artwork in `assets/icons/` (Linux sizes included); the
`.ico` and `.icns` in `assets/app-icon/` come from it through `packaging/icons.sh`, so replace the
artwork there and rerun the script, not each installer. What is still missing is in
`NODALIA_MIGRATION.md`.

## Pull requests

A change is ready when the gate passes, new behaviour has a test that fails without it, UI changes
come with a screenshot taken from the running app, and docs that describe the behaviour are
updated. `main` changes only through pull requests.
