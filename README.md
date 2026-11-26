# Circuito

Circuito is Nodalia's electronics application: schematic capture, circuit simulation, PCB design and
manufacturing outputs, written in Rust with engines of its own (no wrapper around SPICE, KiCad or
anything else). It belongs to the Engineering suite with Forja, Torno, Vortia and Ábaco.
Licence: `MIT OR Apache-2.0`.

## What runs today

Version 0.2 is a set of engines, a command line and a desktop app in which you draw, edit and
simulate circuits, either with the mouse or by writing them as code. The two stay in step: edit the
drawing and the code changes, edit the code and the drawing changes. There is no PCB editor yet,
and nothing here is certified or fit for safety-critical use.

```console
$ circuito sim examples/divider.circuito
* .op  [circuito-sim 0.1.0-dev; dc: newton]
V(IN)   1.000000000e1 V
V(OUT)  6.875000000e0 V
$ circuito sim examples/divider.circ      # the same circuit written as code
$ circuito erc examples/divider.circuito
examples/divider.circuito: 0 error(s), 0 warning(s), 0 note(s)
$ circuito drc examples/sample-board.circuito-pcb
examples/sample-board.circuito-pcb: 0 error(s), 0 warning(s) against generic-2layer-6-6
$ circuito gerber examples/sample-board.circuito-pcb --out out/
```

* **Core** (`circuito-core`): engineering values (`4.7k`, `100n`), source waveforms, the netlist, the
  schematic (`.circuito` JSON v1) with net resolution, and a 19-part library.
* **SPICE** (`circuito-spice`): reads and writes R, C, L, V, I, D, E, G, F, H, K, Q, M, S cards,
  `.model`, `.op`, `.dc`, `.ac`, `.tran`. Unsupported cards and `.include` are errors, never skipped.
* **Simulator** (`circuito-sim`): modified nodal analysis with Newton–Raphson, gmin and source
  stepping; DC operating point, AC small-signal, transient with trapezoidal and backward Euler and
  step control. Models: R, C, L, mutual inductors, independent and controlled sources, diode,
  BJT (Ebers-Moll with Early effect), MOSFET level 1, switch, op-amp.
* **Electrical rules check** (`circuito-erc`): 25 rules with stable codes, JSON output.
* **PCB** (`circuito-pcb`): board model, exact-geometry design-rules check, Gerber RS-274X/X2,
  Excellon drill, pick-and-place and BOM CSV.
* **CLI** (`circuito`): `sim`, `erc`, `netlist`, `drc`, `gerber`; table, CSV or JSON output.
* **Circuit code** (`circuito-script`): a short text form of a schematic, one line per part
  (`R1 1k in out`), read into a drawn schematic with automatic placement and routing, and written
  back from a drawing. Applying a text to a drawing keeps what you placed. See
  [`docs/script.md`](docs/script.md).
* **App** (`circuito-app`, egui with the Nodalia Design Language): a schematic editor. Drag parts
  from the list onto the sheet, draw wires that snap to pins and to other wires, select, move,
  rotate, mirror, copy, paste and delete, edit references, values, models and fields in the
  properties panel, add net labels, power ports and no-connect flags, with unlimited undo and redo
  and a new circuit, save and save as (`.circuito`, or `.circ` as code). A code pane beside the
  drawing shows the circuit as text, highlighted, with the problems of what you type listed by
  line. It also opens SPICE decks (also by drag and drop), lists the rules-check findings, runs
  `.op`, `.ac`, `.tran` and `.dc` on a worker thread with progress and cancel as you edit, and
  shows the operating point and plots with engineering units and a logarithmic frequency axis.
  English and Spanish, light and dark. The rules-check messages come from the engine and are shown
  in English.

What the engines were checked against is written in each crate's README. Simulation results were
compared with analytic solutions and the published model equations, not with another simulator.
The Gerber and Excellon files were read back by our own readers, not opened in an external viewer.

What does not exist yet (see [`ROADMAP.md`](ROADMAP.md)): the PCB editor, hierarchical sheets and
sheet editing in the app (a drawing with several sheets can be viewed, simulated and edited one
sheet at a time but not written as code), a library of your own parts, `.noise`, `.step` and Monte
Carlo, zone fill, autorouting, 3D board, PLC and FPGA tools.

## Install

Every release on [GitHub](https://github.com/getnodalia/circuito/releases) carries these files
(`<version>` is the release, for example `0.1.0`). None of them is signed with a developer
certificate, so the first start asks you to confirm.

| System | File | Notes |
|---|---|---|
| Linux x86_64 | `circuito-<version>-linux-x86_64.AppImage` | Make it executable and run it. |
| | `.deb`, `.rpm`, `.tar.gz` of the same name | `sudo apt install ./circuito-<version>-linux-x86_64.deb`, `sudo dnf install ./circuito-<version>-linux-x86_64.rpm`, or unpack the tarball (`bin/`, `share/`). |
| macOS | `circuito-<version>-macos-universal.dmg` | Intel and Apple silicon. Ad hoc signed, not notarized: open the app with right click, Open. |
| Windows x64 | `circuito-<version>-windows-x64.msi` | Installs for your account or for everyone. SmartScreen warns on the first start (More info, Run anyway). |
| | `circuito-<version>-windows-x64-portable.zip` | Unpack anywhere. |

Each package has the desktop app (`circuito-app`) and the `circuito` command. The macOS command-line
build is also published alone as `circuito-cli-<version>-macos-universal.zip`. Checksums are in
`SHA256SUMS` (Linux), `SHA256SUMS-macos` and `SHA256SUMS-windows`.

The release pipeline is described in [`docs/release-notes/README.md`](docs/release-notes/README.md);
the packaging scripts are in [`packaging/`](packaging/). The Linux packages were built and the
AppImage started on one machine; the macOS and Windows packaging, and the workflows themselves,
run on GitHub for the first time with the first release.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
cargo run -p circuito-cli -- sim examples/rc-lowpass.cir
cargo run -p circuito-app -- examples/divider.circ     # the app (Linux needs libxkbcommon-x11-0 and libegl1)
```

## Layout

| Path | What it is |
|---|---|
| `crates/circuito-core` | Values, netlist, schematic model, part library |
| `crates/circuito-spice` | SPICE netlist reader and writer |
| `crates/circuito-script` | The circuit code: text to drawn schematic, schematic to text, and applying a text to a drawing |
| `crates/circuito-sim` | The simulator (uses `nodalia-numerics` from Ábaco, pinned by revision) |
| `crates/circuito-erc` | Netlist extraction and the electrical rules check |
| `crates/circuito-pcb` | Board model, DRC, Gerber, Excellon, assembly outputs |
| `apps/circuito-cli` | The `circuito` command |
| `apps/circuito` | The egui app (`circuito-app`); its logic is a testable library, the windows are the binary |
| `examples/` | Small circuits and a sample board used by the tests |
| `packaging/` | Installers: Linux (AppImage, deb, rpm, tar.gz), macOS DMG, Windows MSI and zip, and the icon files |
| `assets/icons/` | The app icon: the Kirchhoff portrait artwork and its Linux sizes |
| `assets/app-icon/` | The Windows `.ico` and macOS `.icns` made from it by `packaging/icons.sh` |

Rules for contributors and agents: [`AGENTS.md`](AGENTS.md). Security: [`SECURITY.md`](SECURITY.md).
