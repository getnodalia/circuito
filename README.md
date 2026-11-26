# Circuito

Circuito is Nodalia's electronics application: schematic capture, circuit simulation, PCB design and
manufacturing outputs, written in Rust with engines of its own (no wrapper around SPICE, KiCad or
anything else). It belongs to the Engineering suite with Forja, Torno, Vortia and Ábaco.
Licence: `MIT OR Apache-2.0`.

## What runs today

Version 0.1 is a set of engines, a command line and a first desktop app that opens, checks and
simulates circuits. The app has no schematic editor yet: you cannot draw or change a circuit in it,
and nothing here is certified or fit for safety-critical use.

```console
$ circuito sim examples/divider.circuito
* .op  [circuito-sim 0.1.0-dev; dc: newton]
V(IN)   1.000000000e1 V
V(OUT)  6.875000000e0 V
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
* **App** (`circuito-app`, egui with the Nodalia Design Language): opens a `.circuito` schematic or
  a SPICE deck (also by drag and drop), draws the schematic with pan and zoom, lists the rules-check
  findings (a click selects the part or wire), runs `.op`, `.ac`, `.tran` and `.dc` on a worker
  thread with progress and cancel, and shows the operating point and plots with engineering units
  and a logarithmic frequency axis. English and Spanish, light and dark. The rules-check messages
  come from the engine and are shown in English.

What the engines were checked against is written in each crate's README. Simulation results were
compared with analytic solutions and the published model equations, not with another simulator.
The Gerber and Excellon files were read back by our own readers, not opened in an external viewer.

What does not exist yet (see [`ROADMAP.md`](ROADMAP.md)): the schematic editor (the app only views
what it opens), the PCB editor, `.noise`, `.step` and Monte Carlo, zone fill, autorouting, 3D board, PLC and FPGA tools.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
cargo run -p circuito-cli -- sim examples/rc-lowpass.cir
cargo run -p circuito-app -- examples/rc-lowpass.cir   # the app (Linux needs libxkbcommon-x11-0 and libegl1)
```

## Layout

| Path | What it is |
|---|---|
| `crates/circuito-core` | Values, netlist, schematic model, part library |
| `crates/circuito-spice` | SPICE netlist reader and writer |
| `crates/circuito-sim` | The simulator (uses `nodalia-numerics` from Ábaco, pinned by revision) |
| `crates/circuito-erc` | Netlist extraction and the electrical rules check |
| `crates/circuito-pcb` | Board model, DRC, Gerber, Excellon, assembly outputs |
| `apps/circuito-cli` | The `circuito` command |
| `apps/circuito` | The egui app (`circuito-app`); its logic is a testable library, the windows are the binary |
| `examples/` | Small circuits and a sample board used by the tests |

Rules for contributors and agents: [`AGENTS.md`](AGENTS.md). Security: [`SECURITY.md`](SECURITY.md).
