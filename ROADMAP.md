# Circuito roadmap

Only what runs is described as existing. Levels: planned, experimental, partially functional,
functional, validated (against analytic or published references), production-mature.

| Area | State |
|---|---|
| Values, netlist, schematic model, part library | Functional |
| SPICE reader/writer | Functional for the supported cards; others are refused with a line number |
| DC operating point | Validated against closed forms (dividers, bridge, controlled sources, diode, BJT, MOSFET, op-amp) |
| AC analysis | Validated against transfer functions (RC, series RLC, transformer, small-signal gain) |
| Transient | Validated against closed forms (RC, RL, RLC, LC); measured order 2 (trapezoidal) and 1 (backward Euler) |
| ERC | Functional, 25 rules |
| PCB model and DRC | Functional; boundary-tested at the rule value and 1 nm below |
| Gerber and Excellon output | Experimental: round-trips through our own readers; not opened in an external CAM viewer |
| CLI: `sim`, `erc`, `netlist` | Functional on Linux |
| CLI: `drc`, `gerber` | Functional on Linux; the sample board is DRC-clean, its Gerber files were read back by our own reader only |
| egui app: open, view, rules check, simulate, plots | Partially functional; checked by hand under a virtual display on Linux only |
| Circuit code (`.circ`): text to drawing and back | Functional; round-trips and simulates like the drawn circuit in tests, random text never panics |
| Schematic editor: place, wire, move, rotate, mirror, edit, copy/paste, undo, save | Partially functional; the editing steps are unit-tested, the mouse interactions were checked by hand under a virtual display on Linux only; one sheet at a time |
| PCB editor | Planned |
| `.noise`, `.step`, Monte Carlo, `.ic`/`.nodeset` | Planned |
| Zone fill, autorouting, differential-pair rules | Planned |
| Comparison with another simulator's output | Not done |
| PLC, industrial electrical, FPGA, embedded | Planned |

## Next

1. A PCB view and editor; sheet management and hierarchy in the schematic editor, wire dragging
   with live rerouting, and your own parts.
2. Validate Gerber output with an external viewer and publish the result here.
3. Simulation: `.step`, `.noise`, Gummel-Poon, temperature scaling.
4. PCB editor, zone fill, routing aids.

## Known limitations

See the README of each crate; the important ones: convergence is judged on the solution vector
only; transient output is at the solver's accepted time points; the DRC approximates arcs to 1 µm;
no hierarchical sheets or buses.
